//! MCP support shared by `runa mcp` and the desktop GUI:
//!
//! - [`Policy`]: what AI agents may do (stored in `settings.json`);
//! - the approval bridge: the GUI listens on `127.0.0.1:0` and writes the port and a random
//!   token to `<runa home>/mcp-bridge.json`; the MCP server asks it to approve one exact
//!   operation and gets back a proof bound to that operation;
//! - AI client configuration (Cursor, VS Code, Claude Desktop, Claude Code, Codex,
//!   Windsurf): config file locations, non-destructive merge / removal, install links.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

// ================================================================ policy

pub const MODE_READ_ONLY: &str = "read_only";
pub const MODE_ALLOW_WRITE: &str = "allow_write";
pub const MODES: &[&str] = &[MODE_READ_ONLY, MODE_ALLOW_WRITE];

/// What `runa mcp` lets an AI agent do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Policy {
    /// `read_only` (default) or `allow_write`.
    pub mode: String,
    /// Uninstalls must be approved in the GUI (default on).
    pub confirm_destructive: bool,
    /// Installs and global version changes must be approved in the GUI too (default off).
    pub confirm_writes: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            mode: MODE_READ_ONLY.into(),
            confirm_destructive: true,
            confirm_writes: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Install,
    SetGlobal,
    Uninstall,
}

impl OpKind {
    pub fn destructive(self) -> bool {
        self == OpKind::Uninstall
    }
    pub fn label(self) -> &'static str {
        match self {
            OpKind::Install => "安装",
            OpKind::SetGlobal => "设为全局版本",
            OpKind::Uninstall => "卸载",
        }
    }
}

impl Policy {
    pub fn validate(&self) -> Result<()> {
        if !MODES.contains(&self.mode.as_str()) {
            bail!(
                "未知的 MCP 模式 {:?}，可选 read_only / allow_write",
                self.mode
            );
        }
        Ok(())
    }

    pub fn allows_write(&self) -> bool {
        self.mode == MODE_ALLOW_WRITE
    }

    /// Refuse write operations in read-only mode (unknown modes count as read-only).
    pub fn check(&self, kind: OpKind) -> Result<()> {
        if !self.allows_write() {
            bail!(
                "Runa 的 MCP 当前为只读模式，已拒绝{}。如需允许，请在 Runa 桌面端的“MCP”页面切换为“允许写入”。",
                kind.label()
            );
        }
        Ok(())
    }

    /// Whether `kind` must be approved in the GUI.
    pub fn needs_approval(&self, kind: OpKind) -> bool {
        if kind.destructive() {
            self.confirm_destructive
        } else {
            self.confirm_writes
        }
    }
}

// ================================================================ approval bridge

pub const NOT_RUNNING: &str =
    "此操作需要在 Runa 桌面端确认，但 Runa 未运行。请先打开 Runa 桌面端，然后重试。";

/// Contents of `<home>/mcp-bridge.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeInfo {
    pub port: u16,
    pub token: String,
    pub pid: u32,
}

pub fn bridge_file(home: &Path) -> PathBuf {
    home.join("mcp-bridge.json")
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("系统随机数不可用");
    hex::encode(b)
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// One exact operation an agent wants to perform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Operation {
    pub kind: OpKind,
    pub tool: String,
    pub version: String,
    #[serde(default)]
    pub via: Option<String>,
    /// Uninstall mode (`auto` / `forget` / `purge`).
    #[serde(default)]
    pub mode: Option<String>,
}

/// What the GUI shows in the approval dialog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// Random, single use.
    pub id: String,
    pub op: Operation,
    pub summary: String,
    #[serde(default)]
    pub details: Vec<String>,
    /// MCP client name (from `initialize`), if known.
    #[serde(default)]
    pub client: Option<String>,
}

impl ApprovalRequest {
    pub fn new(
        op: Operation,
        summary: String,
        details: Vec<String>,
        client: Option<String>,
    ) -> Self {
        Self {
            id: random_hex(16),
            op,
            summary,
            details,
            client,
        }
    }

    /// Proof that the bridge owner approved exactly this request (id + operation).
    pub fn proof(&self, token: &str) -> String {
        let op = serde_json::to_string(&self.op).unwrap_or_default();
        sha256_hex(&format!("runa-mcp-approval\n{token}\n{}\n{op}", self.id))
    }
}

#[derive(Serialize, Deserialize)]
struct WireRequest {
    token: String,
    request: ApprovalRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireReply {
    id: String,
    approved: bool,
    #[serde(default)]
    proof: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

const MAX_LINE: u64 = 64 * 1024;

fn read_line(s: &TcpStream) -> Result<String> {
    let mut line = String::new();
    BufReader::new(s.take(MAX_LINE)).read_line(&mut line)?;
    Ok(line)
}

/// Ask the running GUI to approve `req`. Blocks until the user decides or `timeout`.
/// `Ok(())` only for a verified approval of exactly this request.
pub fn request_approval(home: &Path, req: &ApprovalRequest, timeout: Duration) -> Result<()> {
    let info: BridgeInfo = std::fs::read_to_string(bridge_file(home))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| anyhow!(NOT_RUNNING))?;
    let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|_| anyhow!(NOT_RUNNING))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    let msg = serde_json::to_string(&WireRequest {
        token: info.token.clone(),
        request: req.clone(),
    })?;
    s.write_all(msg.as_bytes())?;
    s.write_all(b"\n")?;
    s.flush()?;
    let line = read_line(&s).map_err(|_| anyhow!("等待 Runa 桌面端确认超时，操作未执行"))?;
    if line.trim().is_empty() {
        bail!("Runa 桌面端没有返回确认结果，操作未执行");
    }
    let reply: WireReply =
        serde_json::from_str(&line).context("Runa 桌面端返回了无效的确认结果")?;
    if reply.id != req.id {
        bail!("Runa 桌面端返回的确认与本次操作不符，操作未执行");
    }
    if !reply.approved {
        bail!(
            "{}",
            reply
                .message
                .unwrap_or_else(|| "用户在 Runa 桌面端拒绝了该操作".into())
        );
    }
    let expected = req.proof(&info.token);
    match reply.proof {
        Some(p) if ct_eq(&p, &expected) => Ok(()),
        _ => bail!("Runa 桌面端的确认无法验证，操作未执行"),
    }
}

/// `decide` blocks until the user answers; `Err(message)` rejects with a reason.
pub type Decide = Arc<dyn Fn(ApprovalRequest) -> std::result::Result<(), String> + Send + Sync>;

/// The GUI side of the bridge. The accept loop runs until the process exits.
pub struct Bridge {
    pub info: BridgeInfo,
    pub file: PathBuf,
}

impl Bridge {
    pub fn start(home: &Path, decide: Decide) -> Result<Bridge> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).context("无法启动 MCP 确认服务")?;
        let info = BridgeInfo {
            port: listener.local_addr()?.port(),
            token: random_hex(32),
            pid: std::process::id(),
        };
        let file = bridge_file(home);
        crate::storage::write_atomic(&file, &serde_json::to_vec_pretty(&info)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
        }
        let token = info.token.clone();
        let seen: Arc<Mutex<HashSet<String>>> = Default::default();
        std::thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let (token, seen, decide) = (token.clone(), seen.clone(), decide.clone());
                std::thread::spawn(move || {
                    let _ = serve_conn(conn, &token, &seen, &decide);
                });
            }
        });
        Ok(Bridge { info, file })
    }

    /// Remove the bridge file if it is still ours.
    pub fn remove_file(&self) {
        let ours = std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|t| serde_json::from_str::<BridgeInfo>(&t).ok())
            .is_some_and(|i| i == self.info);
        if ours {
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

fn serve_conn(
    mut s: TcpStream,
    token: &str,
    seen: &Mutex<HashSet<String>>,
    decide: &Decide,
) -> Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let line = read_line(&s)?;
    let w: WireRequest = serde_json::from_str(&line)?;
    if !ct_eq(&w.token, token) {
        bail!("bad token"); // drop silently
    }
    let req = w.request;
    let reply = |approved: bool, message: Option<String>| WireReply {
        id: req.id.clone(),
        approved,
        proof: approved.then(|| req.proof(token)),
        message,
    };
    let fresh = req.id.len() >= 16 && seen.lock().unwrap().insert(req.id.clone());
    let r = if !fresh {
        reply(false, Some("重复的确认请求，已拒绝".into()))
    } else {
        match decide(req.clone()) {
            Ok(()) => reply(true, None),
            Err(m) => reply(false, Some(m)),
        }
    };
    s.write_all(serde_json::to_string(&r)?.as_bytes())?;
    s.write_all(b"\n")?;
    Ok(())
}

// ================================================================ AI clients

pub const SERVER_NAME: &str = "runa";
pub const CLIENT_IDS: &[&str] = &[
    "cursor",
    "vscode",
    "claude-desktop",
    "claude-code",
    "codex",
    "windsurf",
    "opencode",
];

/// Per-user locations used to find client config files.
#[derive(Debug, Clone)]
pub struct ClientEnv {
    pub os: String,
    pub home: PathBuf,
    /// `%APPDATA%` on Windows.
    pub appdata: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` on Linux.
    pub xdg_config: Option<PathBuf>,
    /// `$CODEX_HOME`.
    pub codex_home: Option<PathBuf>,
    /// `$OPENCODE_CONFIG` (custom OpenCode config file).
    pub opencode_config: Option<PathBuf>,
}

impl ClientEnv {
    pub fn current() -> Result<Self> {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Ok(Self {
            os: std::env::consts::OS.into(),
            home: dirs::home_dir().ok_or_else(|| anyhow!("无法确定用户主目录"))?,
            appdata: var("APPDATA"),
            xdg_config: var("XDG_CONFIG_HOME"),
            codex_home: var("CODEX_HOME"),
            opencode_config: var("OPENCODE_CONFIG"),
        })
    }

    fn app_config(&self) -> PathBuf {
        match self.os.as_str() {
            "macos" => self.home.join("Library").join("Application Support"),
            "windows" => self
                .appdata
                .clone()
                .unwrap_or_else(|| self.home.join("AppData").join("Roaming")),
            _ => self
                .xdg_config
                .clone()
                .unwrap_or_else(|| self.home.join(".config")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// JSON object with servers under this top-level key.
    Json(&'static str),
    /// Codex `config.toml` (`[mcp_servers.<name>]`).
    Toml,
}

pub fn client_name(id: &str) -> &'static str {
    match id {
        "cursor" => "Cursor",
        "vscode" => "VS Code",
        "claude-desktop" => "Claude Desktop",
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "windsurf" => "Windsurf",
        "opencode" => "OpenCode",
        _ => "",
    }
}

/// The user-level config file of a client and its format.
pub fn config_file(id: &str, env: &ClientEnv) -> Option<(PathBuf, Format)> {
    let h = &env.home;
    Some(match id {
        "cursor" => (
            h.join(".cursor").join("mcp.json"),
            Format::Json("mcpServers"),
        ),
        "vscode" => (
            env.app_config().join("Code").join("User").join("mcp.json"),
            Format::Json("servers"),
        ),
        "claude-desktop" => (
            env.app_config()
                .join("Claude")
                .join("claude_desktop_config.json"),
            Format::Json("mcpServers"),
        ),
        "claude-code" => (h.join(".claude.json"), Format::Json("mcpServers")),
        "codex" => (
            env.codex_home
                .clone()
                .unwrap_or_else(|| h.join(".codex"))
                .join("config.toml"),
            Format::Toml,
        ),
        "windsurf" => (
            h.join(".codeium").join("windsurf").join("mcp_config.json"),
            Format::Json("mcpServers"),
        ),
        "opencode" => (opencode_file(env), Format::Json("mcp")),
        _ => return None,
    })
}

/// OpenCode global config (https://opencode.ai/docs/config): `$OPENCODE_CONFIG`, else
/// `$XDG_CONFIG_HOME/opencode/opencode.json` (`~/.config` on every OS); an existing
/// `opencode.jsonc` is used when there is no `opencode.json`.
fn opencode_file(env: &ClientEnv) -> PathBuf {
    if let Some(p) = &env.opencode_config {
        return p.clone();
    }
    let dir = env
        .xdg_config
        .clone()
        .unwrap_or_else(|| env.home.join(".config"))
        .join("opencode");
    let json = dir.join("opencode.json");
    let jsonc = dir.join("opencode.jsonc");
    if !json.exists() && jsonc.exists() {
        jsonc
    } else {
        json
    }
}

pub fn server_args() -> Vec<String> {
    vec!["mcp".into()]
}

/// The `runa` entry for a JSON config.
pub fn server_entry(id: &str, exe: &str) -> Value {
    if id == "opencode" {
        let mut cmd = vec![exe.to_string()];
        cmd.extend(server_args());
        return json!({ "type": "local", "command": cmd, "enabled": true });
    }
    let mut v = json!({ "command": exe, "args": server_args() });
    if id == "vscode" {
        v["type"] = json!("stdio");
    }
    v
}

pub fn snippet(id: &str, exe: &str) -> String {
    match config_file(id, &dummy_env()).map(|c| c.1) {
        Some(Format::Toml) => merge_toml("", exe).unwrap_or_default(),
        Some(Format::Json(key)) => {
            serde_json::to_string_pretty(&json!({ key: { SERVER_NAME: server_entry(id, exe) } }))
                .unwrap_or_default()
                + "\n"
        }
        None => String::new(),
    }
}

fn dummy_env() -> ClientEnv {
    ClientEnv {
        os: std::env::consts::OS.into(),
        home: PathBuf::from("~"),
        appdata: None,
        xdg_config: None,
        codex_home: None,
        opencode_config: None,
    }
}

/// `encodeURIComponent`.
pub fn url_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// Cursor install link: `cursor://anysphere.cursor-deeplink/mcp/install?name=…&config=<base64 JSON>`
/// (https://cursor.com/docs/context/mcp/install-links).
pub fn cursor_link(exe: &str) -> String {
    use base64::Engine;
    let cfg = serde_json::to_string(&server_entry("cursor", exe)).unwrap_or_default();
    let b64 = base64::engine::general_purpose::STANDARD.encode(cfg);
    format!(
        "cursor://anysphere.cursor-deeplink/mcp/install?name={SERVER_NAME}&config={}",
        url_encode(&b64)
    )
}

/// VS Code install link: `vscode:mcp/install?<encodeURIComponent(JSON)>`
/// (https://code.visualstudio.com/api/extension-guides/ai/mcp).
pub fn vscode_link(exe: &str) -> String {
    let mut v = server_entry("vscode", exe);
    v["name"] = json!(SERVER_NAME);
    let mut obj = Map::new();
    obj.insert("name".into(), json!(SERVER_NAME));
    for (k, x) in v.as_object().cloned().unwrap_or_default() {
        if k != "name" {
            obj.insert(k, x);
        }
    }
    format!(
        "vscode:mcp/install?{}",
        url_encode(&Value::Object(obj).to_string())
    )
}

fn parse_json_obj(text: &str) -> Result<Map<String, Value>> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => bail!("配置文件的顶层不是 JSON 对象"),
        Err(e) => bail!("配置文件不是合法 JSON（可能含注释）：{e}。请使用“复制配置”手动添加"),
    }
}

/// Insert / replace `<key>.runa` and keep everything else.
pub fn merge_json(text: &str, key: &str, entry: Value) -> Result<String> {
    let mut root = parse_json_obj(text)?;
    let servers = root
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        bail!("配置文件中的 {key} 不是 JSON 对象");
    };
    servers.insert(SERVER_NAME.into(), entry);
    Ok(serde_json::to_string_pretty(&Value::Object(root))? + "\n")
}

/// Remove `<key>.runa`. Returns `None` when there was nothing to remove.
pub fn remove_json(text: &str, key: &str) -> Result<Option<String>> {
    let mut root = parse_json_obj(text)?;
    let removed = root
        .get_mut(key)
        .and_then(|s| s.as_object_mut())
        .and_then(|s| s.remove(SERVER_NAME))
        .is_some();
    if !removed {
        return Ok(None);
    }
    Ok(Some(
        serde_json::to_string_pretty(&Value::Object(root))? + "\n",
    ))
}

pub fn json_has(text: &str, key: &str) -> bool {
    parse_json_obj(text)
        .ok()
        .and_then(|r| r.get(key).and_then(|s| s.get(SERVER_NAME)).cloned())
        .is_some()
}

fn parse_toml(text: &str) -> Result<toml_edit::DocumentMut> {
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|e| anyhow!("配置文件不是合法 TOML：{e}"))
}

/// Insert / replace `[mcp_servers.runa]`, keeping formatting and comments elsewhere.
pub fn merge_toml(text: &str, exe: &str) -> Result<String> {
    use toml_edit::{value, Array, Item, Table};
    let mut doc = parse_toml(text)?;
    if !doc.contains_key("mcp_servers") {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(t));
    }
    let servers = doc["mcp_servers"]
        .as_table_mut()
        .ok_or_else(|| anyhow!("配置文件中的 mcp_servers 不是表"))?;
    let mut t = Table::new();
    t.insert("command", value(exe));
    let mut a = Array::new();
    for x in server_args() {
        a.push(x);
    }
    t.insert("args", value(a));
    servers.insert(SERVER_NAME, Item::Table(t));
    Ok(doc.to_string())
}

pub fn remove_toml(text: &str) -> Result<Option<String>> {
    let mut doc = parse_toml(text)?;
    let removed = doc
        .get_mut("mcp_servers")
        .and_then(|s| s.as_table_mut())
        .and_then(|s| s.remove(SERVER_NAME))
        .is_some();
    Ok(removed.then(|| doc.to_string()))
}

pub fn toml_has(text: &str) -> bool {
    parse_toml(text).ok().is_some_and(|d| {
        d.get("mcp_servers")
            .and_then(|s| s.get(SERVER_NAME))
            .is_some()
    })
}

/// Is `runa` configured in this client's user-level config?
pub fn is_added(id: &str, env: &ClientEnv) -> bool {
    let Some((path, fmt)) = config_file(id, env) else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    match fmt {
        Format::Json(k) => json_has(&text, k),
        Format::Toml => toml_has(&text),
    }
}

/// Write `text` to `path`, first copying an existing file to `<name>.runa-backup-<time>`.
/// Returns the backup path.
pub fn write_with_backup(path: &Path, text: &str) -> Result<Option<PathBuf>> {
    let backup = if path.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".runa-backup-{ts}"));
        let b = path.with_file_name(name);
        std::fs::copy(path, &b).with_context(|| format!("备份 {} 失败", path.display()))?;
        Some(b)
    } else {
        None
    };
    crate::storage::write_atomic(path, text.as_bytes())?;
    Ok(backup)
}

#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
    pub changed: bool,
}

/// Add `runa` to a file-configured client.
pub fn add_to_file(id: &str, env: &ClientEnv, exe: &str) -> Result<FileChange> {
    let (path, fmt) = config_file(id, env).ok_or_else(|| anyhow!("未知客户端 {id}"))?;
    let old = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("读取 {} 失败", path.display())),
    };
    let new = match fmt {
        Format::Json(k) => merge_json(&old, k, server_entry(id, exe))?,
        Format::Toml => merge_toml(&old, exe)?,
    };
    if new == old {
        return Ok(FileChange {
            path,
            backup: None,
            changed: false,
        });
    }
    let backup = write_with_backup(&path, &new)?;
    Ok(FileChange {
        path,
        backup,
        changed: true,
    })
}

/// Remove `runa` from a client's config file (no-op when absent).
pub fn remove_from_file(id: &str, env: &ClientEnv) -> Result<FileChange> {
    let (path, fmt) = config_file(id, env).ok_or_else(|| anyhow!("未知客户端 {id}"))?;
    let unchanged = |path| FileChange {
        path,
        backup: None,
        changed: false,
    };
    let Ok(old) = std::fs::read_to_string(&path) else {
        return Ok(unchanged(path));
    };
    let new = match fmt {
        Format::Json(k) => remove_json(&old, k)?,
        Format::Toml => remove_toml(&old)?,
    };
    match new {
        None => Ok(unchanged(path)),
        Some(n) => {
            let backup = write_with_backup(&path, &n)?;
            Ok(FileChange {
                path,
                backup,
                changed: true,
            })
        }
    }
}

/// Locate the `claude` CLI (PATH, then its default install locations).
pub fn find_claude_cli(env: &ClientEnv) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.push(env.home.join(".local").join("bin"));
    dirs.push(env.home.join(".claude").join("local"));
    dirs.push(env.home.join(".npm-global").join("bin"));
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    if let Some(a) = &env.appdata {
        dirs.push(a.join("npm"));
    }
    dirs.retain(|d| !d.to_string_lossy().contains(".runa"));
    crate::manager::find_exe_in(&dirs, "claude")
}

/// `claude mcp add --scope user runa -- <exe> mcp` (https://code.claude.com/docs/en/mcp).
pub fn claude_add_args(exe: &str) -> Vec<String> {
    let mut a: Vec<String> = ["mcp", "add", "--scope", "user", SERVER_NAME, "--", exe]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.extend(server_args());
    a
}

pub fn claude_remove_args() -> Vec<String> {
    ["mcp", "remove", "--scope", "user", SERVER_NAME]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Display form of a command line (quotes arguments with spaces).
pub fn shell_line(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .map(|a| {
            if a.is_empty() || a.contains([' ', '"', '\'', '&', '(', ')']) {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_defaults_and_checks() {
        let p = Policy::default();
        assert_eq!(p.mode, "read_only");
        assert!(p.confirm_destructive && !p.confirm_writes);
        for k in [OpKind::Install, OpKind::SetGlobal, OpKind::Uninstall] {
            assert!(p.check(k).is_err());
        }
        let w = Policy {
            mode: "allow_write".into(),
            ..Default::default()
        };
        assert!(w.check(OpKind::Uninstall).is_ok());
        assert!(w.needs_approval(OpKind::Uninstall));
        assert!(!w.needs_approval(OpKind::Install));
        let all = Policy {
            confirm_writes: true,
            ..w.clone()
        };
        assert!(all.needs_approval(OpKind::SetGlobal));
        let bogus = Policy {
            mode: "yolo".into(),
            ..Default::default()
        };
        assert!(bogus.validate().is_err() && bogus.check(OpKind::Install).is_err());
        // settings without an `mcp` section keep working and default to read-only
        let s: crate::Settings = serde_json::from_str(r#"{"mirror":"cn"}"#).unwrap();
        assert_eq!(s.mcp, Policy::default());
        let s: crate::Settings = serde_json::from_str(r#"{"mcp":{"mode":"allow_write"}}"#).unwrap();
        assert!(s.mcp.allows_write() && s.mcp.confirm_destructive);
    }

    fn op() -> Operation {
        Operation {
            kind: OpKind::Uninstall,
            tool: "node".into(),
            version: "20.1.0".into(),
            via: None,
            mode: Some("auto".into()),
        }
    }

    #[test]
    fn bridge_roundtrip_is_bound_to_the_operation() {
        let home = tempfile::tempdir().unwrap();
        let req = ApprovalRequest::new(op(), "卸载 node 20.1.0".into(), vec![], None);
        // GUI not running
        let e = request_approval(home.path(), &req, Duration::from_secs(2)).unwrap_err();
        assert!(e.to_string().contains("Runa 未运行"));
        let decided: Arc<Mutex<Vec<ApprovalRequest>>> = Default::default();
        let d2 = decided.clone();
        let bridge = Bridge::start(
            home.path(),
            Arc::new(move |r: ApprovalRequest| {
                d2.lock().unwrap().push(r.clone());
                if r.op.version == "20.1.0" {
                    Ok(())
                } else {
                    Err("用户拒绝".into())
                }
            }),
        )
        .unwrap();
        request_approval(home.path(), &req, Duration::from_secs(5)).unwrap();
        assert_eq!(decided.lock().unwrap()[0], req);
        // the same request id cannot be approved twice
        assert!(request_approval(home.path(), &req, Duration::from_secs(5)).is_err());
        // rejection
        let mut o = op();
        o.version = "18.0.0".into();
        let r2 = ApprovalRequest::new(o, "x".into(), vec![], None);
        let e = request_approval(home.path(), &r2, Duration::from_secs(5)).unwrap_err();
        assert!(e.to_string().contains("用户拒绝"));
        // proofs differ per operation and per id
        assert_ne!(req.proof("t"), r2.proof("t"));
        let mut r3 = req.clone();
        r3.op.mode = Some("purge".into());
        assert_ne!(req.proof("t"), r3.proof("t"));
        // a wrong token gets no answer
        let mut s = TcpStream::connect(("127.0.0.1", bridge.info.port)).unwrap();
        let w = WireRequest {
            token: "nope".into(),
            request: ApprovalRequest::new(op(), "x".into(), vec![], None),
        };
        writeln!(s, "{}", serde_json::to_string(&w).unwrap()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert!(read_line(&s).unwrap_or_default().is_empty());
        bridge.remove_file();
        assert!(!bridge_file(home.path()).exists());
    }

    #[test]
    fn json_merge_preserves_other_entries() {
        let old = r#"{
  "theme": "dark",
  "mcpServers": { "github": { "command": "gh-mcp", "args": [] } }
}"#;
        let new = merge_json(old, "mcpServers", server_entry("cursor", "/x/runa")).unwrap();
        let v: Value = serde_json::from_str(&new).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["github"]["command"], "gh-mcp");
        assert_eq!(v["mcpServers"]["runa"]["command"], "/x/runa");
        assert_eq!(v["mcpServers"]["runa"]["args"], json!(["mcp"]));
        // key order is kept (theme first)
        assert!(new.find("theme").unwrap() < new.find("mcpServers").unwrap());
        assert!(json_has(&new, "mcpServers"));
        // idempotent
        assert_eq!(
            merge_json(&new, "mcpServers", server_entry("cursor", "/x/runa")).unwrap(),
            new
        );
        let removed = remove_json(&new, "mcpServers").unwrap().unwrap();
        let v: Value = serde_json::from_str(&removed).unwrap();
        assert!(v["mcpServers"].get("runa").is_none());
        assert_eq!(v["mcpServers"]["github"]["command"], "gh-mcp");
        assert!(remove_json(&removed, "mcpServers").unwrap().is_none());
        // empty / missing file
        let v: Value =
            serde_json::from_str(&merge_json("", "servers", json!({})).unwrap()).unwrap();
        assert!(v["servers"]["runa"].is_object());
        // invalid input is refused, not overwritten
        assert!(merge_json("// comment\n{}", "servers", json!({})).is_err());
        assert!(merge_json("[1]", "servers", json!({})).is_err());
        assert!(merge_json(r#"{"servers": 3}"#, "servers", json!({})).is_err());
    }

    #[test]
    fn toml_merge_preserves_comments_and_tables() {
        let old = "# my codex config\nmodel = \"o3\"\n\n[mcp_servers.other]\ncommand = \"other\" # keep\n";
        let new = merge_toml(old, r"C:\Program Files\Runa\runa.exe").unwrap();
        assert!(new.contains("# my codex config"));
        assert!(new.contains("command = \"other\" # keep"));
        let d: toml_edit::DocumentMut = new.parse().unwrap();
        assert_eq!(
            d["mcp_servers"]["runa"]["command"].as_str(),
            Some(r"C:\Program Files\Runa\runa.exe")
        );
        assert_eq!(d["mcp_servers"]["runa"]["args"][0].as_str(), Some("mcp"));
        assert!(toml_has(&new));
        let r = remove_toml(&new).unwrap().unwrap();
        assert!(!toml_has(&r) && r.contains("[mcp_servers.other]"));
        assert!(remove_toml(&r).unwrap().is_none());
        assert!(merge_toml("not = [valid", "x").is_err());
        assert!(merge_toml("", "/r").unwrap().contains("[mcp_servers.runa]"));
    }

    fn env_for(os: &str, home: &Path) -> ClientEnv {
        ClientEnv {
            os: os.into(),
            home: home.into(),
            appdata: (os == "windows").then(|| home.join("AppData").join("Roaming")),
            xdg_config: None,
            codex_home: None,
            opencode_config: None,
        }
    }

    #[test]
    fn config_paths_per_os() {
        let h = Path::new("/h");
        let p = |id, os| config_file(id, &env_for(os, h)).unwrap().0;
        assert_eq!(p("cursor", "linux"), h.join(".cursor/mcp.json"));
        assert_eq!(
            p("claude-desktop", "macos"),
            h.join("Library/Application Support/Claude/claude_desktop_config.json")
        );
        assert_eq!(
            p("claude-desktop", "windows"),
            h.join("AppData")
                .join("Roaming")
                .join("Claude")
                .join("claude_desktop_config.json")
        );
        assert_eq!(p("vscode", "linux"), h.join(".config/Code/User/mcp.json"));
        assert_eq!(
            p("vscode", "macos"),
            h.join("Library/Application Support/Code/User/mcp.json")
        );
        assert_eq!(p("codex", "windows"), h.join(".codex").join("config.toml"));
        assert_eq!(
            p("windsurf", "macos"),
            h.join(".codeium/windsurf/mcp_config.json")
        );
        let mut e = env_for("linux", h);
        e.codex_home = Some("/c".into());
        assert_eq!(
            config_file("codex", &e).unwrap().0,
            Path::new("/c/config.toml")
        );
        assert!(config_file("nope", &e).is_none());
    }

    #[test]
    fn add_and_remove_files_with_backup() {
        let t = tempfile::tempdir().unwrap();
        let env = env_for(std::env::consts::OS, t.path());
        for id in [
            "cursor",
            "vscode",
            "claude-desktop",
            "codex",
            "windsurf",
            "opencode",
        ] {
            assert!(!is_added(id, &env));
            let c = add_to_file(id, &env, "/opt/runa/bin/runa").unwrap();
            assert!(c.changed && c.backup.is_none(), "{id}");
            assert!(is_added(id, &env), "{id}");
            // second add: unchanged
            assert!(!add_to_file(id, &env, "/opt/runa/bin/runa").unwrap().changed);
            // a different path updates and backs up the old file
            let c = add_to_file(id, &env, "/new/runa").unwrap();
            let b = c.backup.unwrap();
            assert!(std::fs::read_to_string(&b)
                .unwrap()
                .contains("/opt/runa/bin/runa"));
            let r = remove_from_file(id, &env).unwrap();
            assert!(r.changed && r.backup.is_some());
            assert!(!is_added(id, &env));
            assert!(!remove_from_file(id, &env).unwrap().changed);
        }
        // existing unrelated content survives
        let (p, _) = config_file("cursor", &env).unwrap();
        std::fs::write(&p, r#"{"mcpServers":{"a":{"command":"a"}},"x":1}"#).unwrap();
        add_to_file("cursor", &env, "/r").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["x"], 1);
        assert_eq!(v["mcpServers"]["a"]["command"], "a");
        // a JSONC file is refused untouched
        std::fs::write(&p, "{ // c\n}").unwrap();
        assert!(add_to_file("cursor", &env, "/r").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ // c\n}");
    }

    #[test]
    fn opencode_config() {
        let t = tempfile::tempdir().unwrap();
        let mut env = env_for("macos", t.path());
        let dir = t.path().join(".config").join("opencode");
        assert_eq!(
            config_file("opencode", &env).unwrap().0,
            dir.join("opencode.json")
        );
        // existing .jsonc is used; comments are refused untouched
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("opencode.jsonc"), "{ // mine\n \"model\": \"x\" }").unwrap();
        assert_eq!(
            config_file("opencode", &env).unwrap().0,
            dir.join("opencode.jsonc")
        );
        assert!(add_to_file("opencode", &env, "/r").is_err());
        assert!(std::fs::read_to_string(dir.join("opencode.jsonc"))
            .unwrap()
            .contains("// mine"));
        // a comment-free .jsonc is merged, other keys kept
        std::fs::write(dir.join("opencode.jsonc"), r#"{"$schema":"https://opencode.ai/config.json","mcp":{"other":{"type":"remote","url":"u"}}}"#).unwrap();
        let c = add_to_file("opencode", &env, "/opt/runa").unwrap();
        assert!(c.backup.is_some());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&c.path).unwrap()).unwrap();
        assert_eq!(v["$schema"], "https://opencode.ai/config.json");
        assert_eq!(v["mcp"]["other"]["url"], "u");
        assert_eq!(
            v["mcp"]["runa"],
            json!({"type": "local", "command": ["/opt/runa", "mcp"], "enabled": true})
        );
        assert!(is_added("opencode", &env));
        assert!(remove_from_file("opencode", &env).unwrap().changed);
        assert!(!is_added("opencode", &env));
        // XDG_CONFIG_HOME and OPENCODE_CONFIG
        env.xdg_config = Some(t.path().join("xdg"));
        assert_eq!(
            config_file("opencode", &env).unwrap().0,
            t.path().join("xdg/opencode/opencode.json")
        );
        env.opencode_config = Some(t.path().join("custom.json"));
        assert_eq!(
            config_file("opencode", &env).unwrap().0,
            t.path().join("custom.json")
        );
        assert!(snippet("opencode", "/r").contains("\"type\": \"local\""));
    }

    #[test]
    fn install_links() {
        use base64::Engine;
        let exe = "/Users/a b/.cargo/bin/runa";
        let c = cursor_link(exe);
        let prefix = "cursor://anysphere.cursor-deeplink/mcp/install?name=runa&config=";
        assert!(c.starts_with(prefix));
        let b64 = c[prefix.len()..]
            .replace("%2B", "+")
            .replace("%2F", "/")
            .replace("%3D", "=");
        let cfg: Value = serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(cfg, json!({ "command": exe, "args": ["mcp"] }));
        let v = vscode_link(exe);
        assert!(v.starts_with("vscode:mcp/install?%7B%22name%22%3A%22runa%22"));
        assert!(!v.contains(' '));
        assert!(snippet("codex", exe).contains("[mcp_servers.runa]"));
        assert!(snippet("vscode", exe).contains("\"servers\""));
        assert_eq!(
            claude_add_args("/r"),
            ["mcp", "add", "--scope", "user", "runa", "--", "/r", "mcp"]
        );
        assert_eq!(url_encode("a b&c=é"), "a%20b%26c%3D%C3%A9");
    }
}
