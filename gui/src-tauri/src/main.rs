//! Runa desktop GUI (Tauri v2). All logic lives in `runa-core`; this file only exposes it
//! as IPC commands. Every command runs on a blocking worker thread because core uses
//! blocking HTTP.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use runa_core::activate;
use std::sync::atomic::{AtomicBool, Ordering};

use runa_core::detect::{self, DetectReport, Detected, MigrationAction};
use runa_core::installer::Progress;
use runa_core::manager::ToolStatus;
use runa_core::uninstall::{self, Mode, UninstallPlan};
use runa_core::{Manager, Settings};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))
}

fn manager() -> anyhow::Result<Manager> {
    Manager::load()
}

fn project_dir(dir: Option<String>) -> PathBuf {
    dir.filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
        .or_else(dirs_home)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[derive(Serialize)]
struct ShellInfo {
    shell: String,
    profile: String,
    configured: bool,
}

#[derive(Serialize)]
struct Overview {
    home: String,
    cli_path: String,
    cli_found: bool,
    project_dir: String,
    platform: String,
    settings: Settings,
    tools: Vec<ToolStatus>,
    shells: Vec<ShellInfo>,
    warnings: Vec<String>,
    is_windows: bool,
    default_shell: String,
    hook_active: bool,
    app_version: String,
}

#[tauri::command]
async fn overview(project: Option<String>) -> CmdResult<Overview> {
    blocking(move || {
        let m = manager()?;
        let dir = project_dir(project);
        let cli = activate::cli_path();
        let shells = activate::SHELLS
            .iter()
            .map(|s| ShellInfo {
                shell: s.to_string(),
                profile: activate::profile_path(s)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                configured: activate::profile_has_block(s),
            })
            .collect();
        Ok(Overview {
            home: m.paths.root.display().to_string(),
            cli_found: activate::cli_available(&cli),
            cli_path: cli.display().to_string(),
            project_dir: dir.display().to_string(),
            platform: runa_core::Target::current().key(),
            settings: m.settings.clone(),
            tools: m.status(&dir),
            shells,
            warnings: m.warnings.clone(),
            is_windows: cfg!(windows),
            default_shell: activate::default_shell(),
            hook_active: std::env::var_os("RUNA_PATH_BASE").is_some(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    })
    .await
}

#[tauri::command]
async fn remote_versions(tool: String) -> CmdResult<Vec<runa_core::plugins::RemoteVersion>> {
    blocking(move || {
        let m = manager()?;
        let p = m.plugin(&tool)?;
        p.list_remote(&m.net()?)
    })
    .await
}

#[derive(Serialize)]
struct Usage {
    tool: String,
    version: String,
    bytes: u64,
}

/// Disk usage of every install (computed lazily, can take a moment for JDKs).
#[tauri::command]
async fn disk_usage() -> CmdResult<Vec<Usage>> {
    blocking(|| {
        let m = manager()?;
        let mut out = Vec::new();
        for tool in m.registry.names() {
            for version in m.installed(&tool) {
                let bytes = runa_core::manager::dir_size(&m.paths.install_dir(&tool, &version));
                out.push(Usage {
                    tool: tool.clone(),
                    version,
                    bytes,
                });
            }
        }
        Ok(out)
    })
    .await
}

#[derive(serde::Serialize)]
struct InstallResult {
    version: String,
    /// `runa` or the version manager that installed it.
    via: String,
}

#[tauri::command]
async fn install(
    app: AppHandle,
    tool: String,
    spec: String,
    via: Option<String>,
) -> CmdResult<InstallResult> {
    blocking(move || {
        let m = manager()?;
        let mut last = Instant::now() - Duration::from_secs(1);
        let (v, used) = runa_core::managers::install(&m, &tool, &spec, via.as_deref(), &mut |p| {
            let throttled = matches!(&p, Progress::Downloading { done, total, .. } if !total.map(|t| *done >= t).unwrap_or(false));
            if throttled && last.elapsed() < Duration::from_millis(120) {
                return;
            }
            last = Instant::now();
            let _ = app.emit("install-progress", &p);
        })?;
        if m.global_version(&tool).is_none() {
            m.set_global(&tool, &v)?;
        }
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(InstallResult { version: v, via: used })
    })
    .await
}

/// Which installer a new version of `tool` would use (and the alternatives).
#[tauri::command]
async fn installer_choice(tool: String) -> CmdResult<runa_core::managers::InstallerChoice> {
    blocking(move || {
        let m = manager()?;
        let t = m.plugin(&tool)?.name().to_string();
        Ok(runa_core::managers::installer_choice(
            &runa_core::managers::Probe::current(&m),
            &t,
            m.settings.install_via(&t),
        ))
    })
    .await
}

#[tauri::command]
async fn set_install_via(
    tool: String,
    via: String,
) -> CmdResult<runa_core::managers::InstallerChoice> {
    blocking(move || {
        let mut m = manager()?;
        let t = m.plugin(&tool)?.name().to_string();
        let probe = runa_core::managers::Probe::current(&m);
        if via != "auto" && via != "runa" {
            runa_core::managers::choose(&probe, &t, &via)?;
        }
        m.settings.set_install_via(&t, &via)?;
        m.save_settings()?;
        Ok(runa_core::managers::installer_choice(
            &probe,
            &t,
            m.settings.install_via(&t),
        ))
    })
    .await
}

/// How `name` would be installed (exact command, rc-file edits). Changes nothing.
#[tauri::command]
async fn manager_install_plan(name: String) -> CmdResult<runa_core::bootstrap::Method> {
    blocking(move || {
        let m = manager()?;
        runa_core::bootstrap::plan(&runa_core::managers::Probe::current(&m), &name)
    })
    .await
}

#[derive(Clone, serde::Serialize)]
struct ManagerLine {
    name: String,
    line: String,
}

/// Install version manager `name` (output streamed as `manager-progress`), then make it
/// the installer for `tool`. Returns the refreshed installer choice for `tool`.
#[tauri::command]
async fn install_manager(
    app: AppHandle,
    name: String,
    tool: String,
) -> CmdResult<runa_core::managers::InstallerChoice> {
    blocking(move || {
        let mut m = manager()?;
        let t = m.plugin(&tool)?.name().to_string();
        let probe = runa_core::managers::Probe::current(&m);
        if !runa_core::bootstrap::is_installed(&probe, &name) {
            let method = runa_core::bootstrap::plan(&probe, &name)?;
            let n = name.clone();
            runa_core::bootstrap::install(&probe, &method, Some(&t), &mut |line| {
                let _ = app.emit(
                    "manager-progress",
                    ManagerLine {
                        name: n.clone(),
                        line,
                    },
                );
            })?;
        }
        // re-detect and select it
        let probe = runa_core::managers::Probe::current(&m);
        runa_core::managers::choose(&probe, &t, &name)?;
        m.settings.set_install_via(&t, &name)?;
        m.save_settings()?;
        runa_core::detect::invalidate_cache(&m);
        Ok(runa_core::managers::installer_choice(
            &probe,
            &t,
            m.settings.install_via(&t),
        ))
    })
    .await
}

fn emit_line(app: &AppHandle, name: &str, line: String) {
    let _ = app.emit(
        "manager-progress",
        ManagerLine {
            name: name.to_string(),
            line,
        },
    );
}

/// Install a version manager from the 管理器 page (no installer preference change).
#[tauri::command]
async fn install_manager_only(app: AppHandle, name: String) -> CmdResult<()> {
    blocking(move || {
        let m = manager()?;
        let probe = runa_core::managers::Probe::current(&m);
        let method = runa_core::bootstrap::plan(&probe, &name)?;
        runa_core::bootstrap::install(&probe, &method, None, &mut |l| emit_line(&app, &name, l))?;
        runa_core::detect::invalidate_cache(&m);
        Ok(())
    })
    .await
}

/// Version managers relevant to this platform with status, version, origin, runtimes.
#[tauri::command]
async fn manager_info() -> CmdResult<Vec<runa_core::manager_admin::ManagerInfo>> {
    blocking(move || {
        let m = manager()?;
        Ok(runa_core::manager_admin::info(
            &m,
            &runa_core::managers::Probe::current(&m),
        ))
    })
    .await
}

/// Dry run of upgrading / uninstalling a manager. Changes nothing.
#[tauri::command]
async fn manager_plan(
    name: String,
    action: String,
    keep_data: Option<bool>,
) -> CmdResult<runa_core::manager_admin::ManagerPlan> {
    blocking(move || {
        let m = manager()?;
        runa_core::manager_admin::plan_with(
            &m,
            &runa_core::managers::Probe::current(&m),
            &name,
            &action,
            keep_data.unwrap_or(false),
        )
    })
    .await
}

/// Upgrade / uninstall a manager (re-planned here, output as `manager-progress`).
#[tauri::command]
async fn manager_execute(
    app: AppHandle,
    name: String,
    action: String,
    keep_data: Option<bool>,
) -> CmdResult<runa_core::manager_admin::ExecReport> {
    blocking(move || {
        let m = manager()?;
        let probe = runa_core::managers::Probe::current(&m);
        let plan = runa_core::manager_admin::plan_with(
            &m,
            &probe,
            &name,
            &action,
            keep_data.unwrap_or(false),
        )?;
        let rep = runa_core::manager_admin::execute(&m, &probe, &plan, &mut |l| {
            emit_line(&app, &name, l)
        })?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(rep)
    })
    .await
}

/// What uninstalling would do (shown in the confirmation dialog). Changes nothing.
#[tauri::command]
async fn uninstall_plan(tool: String, version: String, mode: Mode) -> CmdResult<UninstallPlan> {
    blocking(move || uninstall::plan(&manager()?, &tool, &version, mode)).await
}

/// `mode`: `auto` (downloads), `forget` (adopted: remove management only) or `purge`
/// (adopted: complete uninstall through the owning tool).
#[tauri::command]
async fn uninstall(tool: String, version: String, mode: Option<Mode>) -> CmdResult<()> {
    blocking(move || {
        let m = manager()?;
        let plan = uninstall::plan(&m, &tool, &version, mode.unwrap_or(Mode::Auto))?;
        uninstall::execute(&m, &plan)?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(())
    })
    .await
}

/// Enable / disable a version; returns warnings (global version, project requirement).
#[tauri::command]
async fn set_disabled(
    tool: String,
    version: String,
    disabled: bool,
    project: Option<String>,
) -> CmdResult<Vec<String>> {
    blocking(move || {
        let m = manager()?;
        let w = m.set_disabled(&tool, &version, disabled, &project_dir(project))?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(w)
    })
    .await
}

#[tauri::command]
async fn set_global(tool: String, version: String) -> CmdResult<String> {
    blocking(move || {
        Ok(manager()?
            .set_global(&tool, &version)?
            .display()
            .to_string())
    })
    .await
}

#[tauri::command]
async fn set_local(project: String, tool: String, version: String) -> CmdResult<String> {
    blocking(move || {
        let dir = PathBuf::from(&project);
        if !dir.is_dir() {
            anyhow::bail!("项目目录不存在: {project}");
        }
        Ok(manager()?
            .set_local(&dir, &tool, &version)?
            .display()
            .to_string())
    })
    .await
}

#[tauri::command]
async fn unset_local(project: String, tool: String) -> CmdResult<()> {
    blocking(move || manager()?.unset(&PathBuf::from(project).join(".tool-versions"), &tool)).await
}

#[tauri::command]
async fn detect_runtimes() -> CmdResult<Vec<Detected>> {
    blocking(|| Ok(detect::detect_cached(&manager()?).items)).await
}

/// Cached results immediately (no scan unless there is no cache at all). `stale` tells
/// the page to expect a `detect-updated` event from [`detect_check`].
#[tauri::command]
async fn detect_cached() -> CmdResult<DetectReport> {
    blocking(|| {
        let m = manager()?;
        Ok(detect::cached_only(&m).unwrap_or_else(|| detect::detect_cached(&m)))
    })
    .await
}

/// Force a rescan.
#[tauri::command]
async fn detect_refresh() -> CmdResult<DetectReport> {
    blocking(|| Ok(detect::detect_fresh(&manager()?))).await
}

static CHECKING: AtomicBool = AtomicBool::new(false);

/// Background check (app start / window focus): rescan only if the fingerprint changed
/// or the cache expired, then emit `detect-updated`. Returns whether it rescanned.
#[tauri::command]
async fn detect_check(app: AppHandle) -> CmdResult<bool> {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    let r = blocking(move || {
        let m = manager()?;
        let stale = detect::cached_only(&m).map(|c| c.stale).unwrap_or(true);
        if !stale {
            return Ok(false);
        }
        let rep = detect::detect_fresh(&m);
        let _ = app.emit("detect-updated", &rep);
        Ok(true)
    })
    .await;
    CHECKING.store(false, Ordering::SeqCst);
    r
}

/// Convert legacy symlink adoptions to registry receipts (`dry_run` only reports).
#[tauri::command]
async fn migrate_adoptions(dry_run: bool) -> CmdResult<Vec<MigrationAction>> {
    blocking(move || {
        let m = manager()?;
        let acts = detect::migrate(&m, dry_run)?;
        if !dry_run {
            let _ = activate::reshim(&m, &activate::cli_path());
        }
        Ok(acts)
    })
    .await
}

#[derive(serde::Serialize)]
struct AdoptResult {
    version: String,
    set_global: bool,
}

#[tauri::command]
async fn adopt(item: Detected, name: Option<String>) -> CmdResult<AdoptResult> {
    blocking(move || {
        let m = manager()?;
        let r = detect::adopt_detected(&m, &item, name.as_deref().filter(|s| !s.is_empty()))?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(AdoptResult {
            version: r.0,
            set_global: r.1,
        })
    })
    .await
}

#[tauri::command]
async fn adopt_all() -> CmdResult<Vec<detect::AdoptOutcome>> {
    blocking(move || {
        let m = manager()?;
        let items = detect::detect_cached(&m).items;
        let r = detect::adopt_all(&m, &items);
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(r)
    })
    .await
}

#[tauri::command]
async fn set_mirror(mode: String) -> CmdResult<Settings> {
    blocking(move || {
        let mut m = manager()?;
        m.settings.set_mirror(&mode)?;
        m.save_settings()?;
        Ok(m.settings.clone())
    })
    .await
}

#[tauri::command]
async fn set_proxy(value: String) -> CmdResult<Settings> {
    blocking(move || {
        let mut m = manager()?;
        m.settings.set_proxy(&value)?;
        m.save_settings()?;
        Ok(m.settings.clone())
    })
    .await
}

#[tauri::command]
async fn setup_shell(shell: String) -> CmdResult<String> {
    blocking(move || {
        let m = manager()?;
        let cli = activate::cli_path();
        if !activate::cli_available(&cli) {
            anyhow::bail!("未找到 runa 命令行程序，请先把 runa 放到 PATH 中或与 GUI 同目录");
        }
        let p = activate::setup_profile(&m, &shell, &cli)?;
        activate::reshim(&m, &cli)?;
        Ok(p.display().to_string())
    })
    .await
}

#[tauri::command]
async fn remove_shell(shell: String) -> CmdResult<Option<String>> {
    blocking(move || Ok(activate::remove_profile(&shell)?.map(|p| p.display().to_string()))).await
}

#[tauri::command]
async fn add_user_path() -> CmdResult<bool> {
    blocking(|| {
        let m = manager()?;
        activate::reshim(&m, &activate::cli_path())?;
        activate::add_shims_to_user_path(&m)
    })
    .await
}

#[tauri::command]
async fn reshim() -> CmdResult<usize> {
    blocking(|| Ok(activate::reshim(&manager()?, &activate::cli_path())?.len())).await
}

// ---------------------------------------------------------------- MCP

use runa_core::mcp::{self as core_mcp, ApprovalRequest, Bridge, ClientEnv, Policy};
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use tauri::Manager as _;

#[derive(Default)]
struct McpState {
    pending: Mutex<HashMap<String, (ApprovalRequest, mpsc::Sender<bool>)>>,
    bridge: Mutex<Option<Bridge>>,
    bridge_error: Mutex<Option<String>>,
}

#[derive(Serialize)]
struct McpClientView {
    id: String,
    name: String,
    /// `deeplink`, `file` or `cli`.
    method: String,
    config_path: Option<String>,
    added: bool,
    snippet: String,
    link: Option<String>,
    /// Claude Code: the `claude` CLI that `一键添加` runs.
    cli: Option<String>,
    command_line: Option<String>,
}

#[derive(Serialize)]
struct McpStatus {
    cli_path: String,
    cli_found: bool,
    policy: Policy,
    bridge_running: bool,
    bridge_error: Option<String>,
    clients: Vec<McpClientView>,
}

fn cli_exe() -> anyhow::Result<String> {
    let cli = activate::cli_path();
    if !activate::cli_available(&cli) {
        anyhow::bail!(
            "未找到 runa 命令行程序，无法配置 MCP。请先把 runa 放到 PATH 中或与 Runa 同目录"
        );
    }
    let abs = std::fs::canonicalize(&cli).unwrap_or(cli);
    Ok(runa_core::platform::strip_verbatim(abs)
        .display()
        .to_string())
}

fn client_views(exe: &str) -> anyhow::Result<Vec<McpClientView>> {
    let env = ClientEnv::current()?;
    let claude = core_mcp::find_claude_cli(&env);
    Ok(core_mcp::CLIENT_IDS
        .iter()
        .map(|id| {
            let (method, link) = match *id {
                "cursor" => ("deeplink", Some(core_mcp::cursor_link(exe))),
                "vscode" => ("deeplink", Some(core_mcp::vscode_link(exe))),
                "claude-code" => ("cli", None),
                _ => ("file", None),
            };
            let line = core_mcp::shell_line("claude", &core_mcp::claude_add_args(exe));
            McpClientView {
                id: id.to_string(),
                name: core_mcp::client_name(id).to_string(),
                method: method.into(),
                config_path: core_mcp::config_file(id, &env).map(|c| c.0.display().to_string()),
                added: core_mcp::is_added(id, &env),
                snippet: if *id == "claude-code" {
                    format!(
                        "{line}\n\n# 或写入 ~/.claude.json 的 mcpServers：\n{}",
                        core_mcp::snippet(id, exe)
                    )
                } else {
                    core_mcp::snippet(id, exe)
                },
                link,
                cli: (*id == "claude-code")
                    .then(|| claude.as_ref().map(|p| p.display().to_string()))
                    .flatten(),
                command_line: (*id == "claude-code").then_some(line),
            }
        })
        .collect())
}

#[tauri::command]
async fn mcp_status(state: tauri::State<'_, Arc<McpState>>) -> CmdResult<McpStatus> {
    let bridge_running = state.bridge.lock().unwrap().is_some();
    let bridge_error = state.bridge_error.lock().unwrap().clone();
    blocking(move || {
        let m = manager()?;
        let cli = activate::cli_path();
        let exe = cli_exe().unwrap_or_else(|_| cli.display().to_string());
        Ok(McpStatus {
            cli_found: activate::cli_available(&cli),
            cli_path: exe.clone(),
            policy: m.settings.mcp.clone(),
            bridge_running,
            bridge_error,
            clients: client_views(&exe)?,
        })
    })
    .await
}

#[tauri::command]
async fn mcp_set_policy(policy: Policy) -> CmdResult<Policy> {
    blocking(move || {
        policy.validate()?;
        let mut m = manager()?;
        m.settings.mcp = policy;
        m.save_settings()?;
        Ok(m.settings.mcp.clone())
    })
    .await
}

fn open_url(url: &str) -> anyhow::Result<()> {
    let mut c = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let st = c.arg(url).status()?;
    if !st.success() {
        anyhow::bail!("无法打开链接，请确认已安装对应的客户端；也可以使用“复制配置”手动添加");
    }
    Ok(())
}

fn run_claude(cli: &std::path::Path, args: &[String]) -> anyhow::Result<String> {
    let out = std::process::Command::new(cli)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        anyhow::bail!("claude {} 失败：{}", args.join(" "), text.trim());
    }
    Ok(text.trim().to_string())
}

/// 一键添加. Returns a message for the toast.
#[tauri::command]
async fn mcp_add(id: String) -> CmdResult<String> {
    blocking(move || {
        let exe = cli_exe()?;
        let env = ClientEnv::current()?;
        let name = core_mcp::client_name(&id);
        match id.as_str() {
            "cursor" => {
                open_url(&core_mcp::cursor_link(&exe))?;
                Ok("已打开 Cursor，请在 Cursor 中确认安装".into())
            }
            "vscode" => {
                open_url(&core_mcp::vscode_link(&exe))?;
                Ok("已打开 VS Code，请在 VS Code 中确认安装".into())
            }
            "claude-code" => {
                let cli = core_mcp::find_claude_cli(&env).ok_or_else(|| {
                    anyhow::anyhow!("未找到 claude 命令，请使用“复制配置”手动添加")
                })?;
                // replace an existing entry (add fails when the name exists)
                let _ = run_claude(&cli, &core_mcp::claude_remove_args());
                run_claude(&cli, &core_mcp::claude_add_args(&exe))?;
                Ok("已通过 claude mcp add 添加到 Claude Code（用户级）".into())
            }
            "claude-desktop" | "codex" | "windsurf" => {
                let c = core_mcp::add_to_file(&id, &env, &exe)?;
                if !c.changed {
                    return Ok(format!("{name} 已配置 runa，无需修改"));
                }
                let b = c
                    .backup
                    .map(|b| format!("，原文件已备份到 {}", b.display()))
                    .unwrap_or_default();
                Ok(format!("已写入 {}{b}。请重启 {name}", c.path.display()))
            }
            _ => anyhow::bail!("未知客户端 {id}"),
        }
    })
    .await
}

#[tauri::command]
async fn mcp_remove(id: String) -> CmdResult<String> {
    blocking(move || {
        let env = ClientEnv::current()?;
        if id == "claude-code" {
            let cli = core_mcp::find_claude_cli(&env).ok_or_else(|| {
                anyhow::anyhow!("未找到 claude 命令，请运行 claude mcp remove --scope user runa")
            })?;
            run_claude(&cli, &core_mcp::claude_remove_args())?;
            return Ok("已从 Claude Code 移除 runa".into());
        }
        let c = core_mcp::remove_from_file(&id, &env)?;
        if !c.changed {
            return Ok("配置文件中没有 runa".into());
        }
        let b = c
            .backup
            .map(|b| format!("，原文件已备份到 {}", b.display()))
            .unwrap_or_default();
        Ok(format!("已从 {} 移除 runa{b}", c.path.display()))
    })
    .await
}

#[tauri::command]
fn mcp_pending(state: tauri::State<'_, Arc<McpState>>) -> Vec<ApprovalRequest> {
    state
        .pending
        .lock()
        .unwrap()
        .values()
        .map(|(r, _)| r.clone())
        .collect()
}

#[tauri::command]
fn mcp_respond(
    state: tauri::State<'_, Arc<McpState>>,
    id: String,
    approved: bool,
) -> CmdResult<()> {
    let tx = state.pending.lock().unwrap().remove(&id).map(|(_, tx)| tx);
    match tx {
        Some(tx) => {
            let _ = tx.send(approved);
            Ok(())
        }
        None => Err("该请求已失效（超时或已处理）".into()),
    }
}

fn start_bridge(app: &AppHandle, state: &Arc<McpState>) {
    let home = match manager() {
        Ok(m) => m.paths.home.clone(),
        Err(e) => {
            *state.bridge_error.lock().unwrap() = Some(err(e));
            return;
        }
    };
    let (app2, st) = (app.clone(), state.clone());
    let decide: core_mcp::Decide = Arc::new(move |req: ApprovalRequest| {
        let id = req.id.clone();
        let (tx, rx) = mpsc::channel();
        st.pending
            .lock()
            .unwrap()
            .insert(id.clone(), (req.clone(), tx));
        let _ = app2.emit("mcp-approval", &req);
        if let Some(w) = app2.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        let r = rx.recv_timeout(Duration::from_secs(290));
        st.pending.lock().unwrap().remove(&id);
        let _ = app2.emit("mcp-approval-done", &id);
        match r {
            Ok(true) => Ok(()),
            Ok(false) => Err("用户在 Runa 桌面端拒绝了该操作".into()),
            Err(_) => Err("等待确认超时，操作未执行".into()),
        }
    });
    match Bridge::start(&home, decide) {
        Ok(b) => *state.bridge.lock().unwrap() = Some(b),
        Err(e) => *state.bridge_error.lock().unwrap() = Some(err(e)),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            overview,
            remote_versions,
            disk_usage,
            install,
            installer_choice,
            set_install_via,
            manager_install_plan,
            install_manager,
            install_manager_only,
            manager_info,
            manager_plan,
            manager_execute,
            uninstall,
            uninstall_plan,
            set_disabled,
            set_global,
            set_local,
            unset_local,
            detect_runtimes,
            detect_cached,
            detect_refresh,
            detect_check,
            migrate_adoptions,
            adopt,
            adopt_all,
            set_mirror,
            set_proxy,
            setup_shell,
            remove_shell,
            add_user_path,
            reshim,
            mcp_status,
            mcp_set_policy,
            mcp_add,
            mcp_remove,
            mcp_pending,
            mcp_respond
        ])
        .setup(|app| {
            let state = Arc::new(McpState::default());
            start_bridge(app.handle(), &state);
            app.manage(state);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("启动 Runa 失败")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(s) = app.try_state::<Arc<McpState>>() {
                    if let Some(b) = s.bridge.lock().unwrap().as_ref() {
                        b.remove_file();
                    }
                }
            }
        });
}
