//! Detect runtimes already installed on the machine and adopt them.
//!
//! Detection scans PATH, JAVA_HOME / GOROOT, well-known directories and the stores of
//! common version managers (nvm, pyenv, sdkman, asdf, fnm, Homebrew Cellar, ...). Results
//! are cached in `~/.runa/cache/detect.json` together with a fingerprint of everything
//! that was scanned, see [`detect_cached`].
//!
//! Adoption is registry based: a receipt records the real home and bin dir; nothing is
//! linked, moved or modified.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::installer::{self, Receipt};
use crate::manager::Manager;
use crate::platform;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Detected {
    pub tool: String,
    pub version: String,
    /// Executable as found (may be a symlink, e.g. /usr/bin/java).
    pub exe: PathBuf,
    /// Directory the executable was found in.
    pub found_in: PathBuf,
    /// Runtime root guessed from the resolved executable (`<root>/bin/<exe>`).
    pub home: PathBuf,
    /// Where it was discovered: PATH, JAVA_HOME, GOROOT, or a well-known directory.
    pub via: String,
    /// Who owns the installation, see [`classify`].
    #[serde(default)]
    pub provider: String,
    /// Homebrew formula (`node`, `openjdk@17`, `python@3.12`) for `provider == "homebrew"`.
    #[serde(default)]
    pub formula: Option<String>,
    /// Already adopted into runa (computed on every read, never taken from the cache).
    #[serde(default)]
    pub adopted: bool,
    /// Version name it was adopted as.
    #[serde(default)]
    pub adopted_as: Option<String>,
    /// The adopted version is disabled.
    #[serde(default)]
    pub disabled: bool,
}

fn candidate_dirs(m: &Manager, tool: &str) -> Vec<(PathBuf, String)> {
    let mut v: Vec<(PathBuf, String)> = m
        .system_path()
        .into_iter()
        .map(|p| (p, "PATH".to_string()))
        .collect();
    let env_root = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
    };
    match tool {
        "java" => {
            if let Some(h) = env_root("JAVA_HOME") {
                v.push((h.join("bin"), "JAVA_HOME".into()));
            }
            let roots: &[&str] = if cfg!(target_os = "macos") {
                &["/Library/Java/JavaVirtualMachines"]
            } else if cfg!(windows) {
                &[
                    "C:\\Program Files\\Java",
                    "C:\\Program Files\\Eclipse Adoptium",
                    "C:\\Program Files\\Microsoft",
                ]
            } else {
                &["/usr/lib/jvm"]
            };
            for r in roots {
                if let Ok(rd) = std::fs::read_dir(r) {
                    for e in rd.flatten() {
                        let p = e.path();
                        let home = if cfg!(target_os = "macos") {
                            p.join("Contents").join("Home")
                        } else {
                            p
                        };
                        v.push((home.join("bin"), "常见目录".into()));
                    }
                }
            }
        }
        "go" => {
            if let Some(h) = env_root("GOROOT") {
                v.push((h.join("bin"), "GOROOT".into()));
            }
            for r in ["/usr/local/go", "/usr/lib/go", "C:\\Program Files\\Go"] {
                v.push((PathBuf::from(r).join("bin"), "常见目录".into()));
            }
        }
        _ => {}
    }
    if let Some(h) = dirs::home_dir() {
        v.extend(store_bin_dirs(&h, tool, &brew_cellars()));
    }
    v
}

fn brew_cellars() -> Vec<PathBuf> {
    [
        "/opt/homebrew/Cellar",
        "/usr/local/Cellar",
        "/home/linuxbrew/.linuxbrew/Cellar",
    ]
    .iter()
    .map(PathBuf::from)
    .collect()
}

/// Real (non-symlink, non-hidden) child directories. Symlinked aliases such as sdkman's
/// `current` or nvm/pyenv aliases are skipped so each runtime is seen once.
fn real_children(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| {
            std::fs::symlink_metadata(e.path())
                .map(|m| m.is_dir() && !m.file_type().is_symlink())
                .unwrap_or(false)
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// Version-manager stores: `(store dir, label, suffix from version dir to runtime home)`.
fn store_roots(user_home: &Path, tool: &str) -> Vec<(PathBuf, &'static str, &'static str)> {
    let h = user_home;
    let fnm = [
        h.join(".local/share/fnm/node-versions"),
        h.join("Library/Application Support/fnm/node-versions"),
    ];
    match tool {
        "node" => {
            let mut v = vec![
                (h.join(".nvm/versions/node"), "nvm", ""),
                (h.join(".asdf/installs/nodejs"), "asdf", ""),
            ];
            v.extend(fnm.into_iter().map(|p| (p, "fnm", "installation")));
            v
        }
        "python" => vec![
            (h.join(".pyenv/versions"), "pyenv", ""),
            (h.join(".asdf/installs/python"), "asdf", ""),
        ],
        "java" => vec![
            (h.join(".sdkman/candidates/java"), "sdkman", ""),
            (h.join(".asdf/installs/java"), "asdf", ""),
            (
                h.join("Library/Java/JavaVirtualMachines"),
                "常见目录",
                "Contents/Home",
            ),
        ],
        "go" => vec![
            (h.join(".sdkman/candidates/go"), "sdkman", ""),
            (h.join(".asdf/installs/golang"), "asdf", "go"),
        ],
        _ => vec![],
    }
}

/// Homebrew formulae providing `tool` (with the suffix from keg to runtime home).
fn brew_formula_matches(tool: &str, formula: &str) -> Option<&'static str> {
    let ok = |base: &str| formula == base || formula.starts_with(&format!("{base}@"));
    match tool {
        "node" if ok("node") => Some(""),
        "python" if formula.starts_with("python@") => Some(""),
        "java" if ok("openjdk") => Some("libexec/openjdk.jdk/Contents/Home"),
        "go" if ok("go") => Some(""),
        _ => None,
    }
}

/// Directories whose contents decide what detection finds for `tool` (fingerprint input).
fn store_scan_dirs(user_home: &Path, tool: &str, cellars: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = store_roots(user_home, tool)
        .into_iter()
        .map(|(p, _, _)| p)
        .collect();
    for c in cellars {
        v.push(c.clone());
        for f in real_children(c) {
            let name = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if brew_formula_matches(tool, &name).is_some() {
                v.push(f);
            }
        }
    }
    v
}

/// Candidate bin dirs inside version-manager stores and Homebrew kegs.
fn store_bin_dirs(user_home: &Path, tool: &str, cellars: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for (root, label, suffix) in store_roots(user_home, tool) {
        for d in real_children(&root) {
            let home = if suffix.is_empty() { d } else { d.join(suffix) };
            out.push((home.join("bin"), label.to_string()));
        }
    }
    for c in cellars {
        for f in real_children(c) {
            let name = f
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let Some(suffix) = brew_formula_matches(tool, &name) else {
                continue;
            };
            for keg in real_children(&f) {
                let home = if suffix.is_empty() {
                    keg
                } else {
                    keg.join(suffix)
                };
                out.push((home.join("bin"), "homebrew".to_string()));
            }
        }
    }
    out
}

/// Classify who owns a runtime home: `(provider, homebrew formula)`.
///
/// `homebrew` (inside a Cellar), `nvm`, `pyenv`, `sdkman`, `asdf`, `fnm`, `system`
/// (/usr, /System, Xcode / CommandLineTools) or `manual` (any other plain directory).
pub fn classify(home: &Path) -> (String, Option<String>) {
    let s = home.to_string_lossy().replace('\\', "/");
    let comps: Vec<&str> = s.split('/').collect();
    if let Some(i) = comps.iter().position(|c| *c == "Cellar") {
        if let Some(f) = comps.get(i + 1).filter(|f| !f.is_empty()) {
            return ("homebrew".into(), Some(f.to_string()));
        }
    }
    let has = |m: &str| s.contains(m);
    let p = if has("/.nvm/versions/node/") {
        "nvm"
    } else if has("/.pyenv/versions/") {
        "pyenv"
    } else if has("/.sdkman/candidates/") {
        "sdkman"
    } else if has("/.asdf/installs/") {
        "asdf"
    } else if has("/fnm/node-versions/") {
        "fnm"
    } else if s == "/usr"
        || (s.starts_with("/usr/") && !s.starts_with("/usr/local/"))
        || s.starts_with("/System/")
        || s.starts_with("/Library/Developer/CommandLineTools")
        || (s.starts_with("/Applications/Xcode") && s.contains(".app/"))
    {
        "system"
    } else {
        "manual"
    };
    (p.into(), None)
}

/// Path segments after `marker` (e.g. `/.nvm/versions/node/` -> `["v20.1.0", "bin"]`).
pub(crate) fn segments_after<'a>(s: &'a str, marker: &str) -> Vec<&'a str> {
    match s.find(marker) {
        Some(i) => s[i + marker.len()..]
            .split('/')
            .filter(|x| !x.is_empty())
            .collect(),
        None => vec![],
    }
}

fn run_version(exe: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .env("RUNA_DISABLE_SHIM", "1")
        .output()
        .ok()?;
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    s.push('\n');
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(s)
}

/// Scan PATH, tool env vars, well-known directories and version-manager stores.
/// Always runs a fresh scan (and refreshes the cache); see [`detect_cached`].
pub fn detect(m: &Manager) -> Vec<Detected> {
    detect_fresh(m).items
}

fn scan(m: &Manager) -> Vec<Detected> {
    let mut found: Vec<Detected> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut homes: Vec<PathBuf> = Vec::new();
    for plugin in m.registry.all() {
        let tool = plugin.name().to_string();
        let exes = plugin.detect_exes();
        if exes.is_empty() {
            continue;
        }
        for (dir, via) in candidate_dirs(m, &tool) {
            if m.paths.contains(&dir) {
                continue;
            }
            for exe in &exes {
                let Some(p) = crate::manager::find_exe_in(std::slice::from_ref(&dir), exe) else {
                    continue;
                };
                let real = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                if seen.contains(&real) || m.paths.contains(&real) {
                    continue;
                }
                seen.push(real.clone());
                let Some(out) = run_version(&p, &plugin.version_args()) else {
                    continue;
                };
                let Some(version) = plugin.parse_version_output(&out) else {
                    continue;
                };
                // Ask the runtime where it really lives. This sees through launcher stubs
                // (macOS /usr/bin/java, /usr/bin/python3) and version-manager shims
                // (pyenv, asdf, ...) whose own location says nothing about the runtime.
                let Some(home) = query_home(&tool, &p).or_else(|| guess_home(&real, exe)) else {
                    continue;
                };
                if homes.contains(&home) || m.paths.contains(&home) {
                    continue;
                }
                homes.push(home.clone());
                let bin_dir = real_bin_dir(&home, exe).unwrap_or_else(|| dir.clone());
                let (provider, formula) = classify(&home);
                found.push(Detected {
                    tool: tool.clone(),
                    version,
                    exe: p,
                    found_in: bin_dir,
                    home,
                    via: via.clone(),
                    provider,
                    formula,
                    adopted: false,
                    adopted_as: None,
                    disabled: false,
                });
                break; // python3 found -> skip python in the same dir
            }
        }
    }
    found
}

/// Ask the runtime itself for its installation root. Works through stubs and shims.
pub(crate) fn query_home(tool: &str, exe: &Path) -> Option<PathBuf> {
    let (args, key): (&[&str], Option<&str>) = match tool {
        "java" => (
            &["-XshowSettings:properties", "-version"],
            Some("java.home = "),
        ),
        "python" => (&["-c", "import sys; print(sys.base_prefix)"], None),
        "node" => (&["-p", "process.execPath"], None),
        "go" => (&["env", "GOROOT"], None),
        _ => return None,
    };
    let out = run_version(exe, args)?;
    let line = match key {
        Some(k) => out
            .lines()
            .find_map(|l| l.trim().strip_prefix(k))?
            .trim()
            .to_string(),
        None => out.lines().next()?.trim().to_string(),
    };
    if line.is_empty() {
        return None;
    }
    let mut p = PathBuf::from(line);
    if tool == "java" && p.ends_with("jre") {
        // Java 8 reports <jdk>/jre; the JDK root is one level up
        if let Some(parent) = p.parent().filter(|d| d.join("bin").join("java").exists()) {
            p = parent.to_path_buf();
        }
    }
    if tool == "node" {
        // execPath is the binary: <root>/bin/node
        p = std::fs::canonicalize(&p)
            .ok()?
            .parent()?
            .parent()?
            .to_path_buf();
    }
    let p = std::fs::canonicalize(&p).unwrap_or(p);
    p.is_dir().then_some(p)
}

/// Fallback: `<root>/bin/<exe>`, accepted only when that layout really exists, so a
/// stub such as /usr/bin/java never turns /usr into a "JDK".
pub(crate) fn guess_home(real: &Path, exe: &str) -> Option<PathBuf> {
    let home = real.parent()?.parent()?.to_path_buf();
    if is_system_prefix(&home) {
        return None;
    }
    real_bin_dir(&home, exe).map(|_| home)
}

pub(crate) fn real_bin_dir(home: &Path, exe: &str) -> Option<PathBuf> {
    [home.join("bin"), home.to_path_buf()]
        .into_iter()
        .find(|d| crate::manager::find_exe_in(std::slice::from_ref(d), exe).is_some())
}

/// Shared system prefixes are never a runtime root by themselves.
pub(crate) fn is_system_prefix(p: &Path) -> bool {
    matches!(
        p.to_str(),
        Some("/" | "/usr" | "/usr/local" | "/opt/homebrew" | "/opt/local")
    )
}

// ------------------------------------------------------------------ cache

/// Cached detection results older than this are re-scanned even if nothing changed.
pub const CACHE_MAX_AGE_SECS: u64 = 24 * 60 * 60;
const CACHE_SCHEMA: u32 = 1;

/// Everything detection depends on. If it is unchanged the cached result is reused.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    pub path: String,
    pub java_home: String,
    pub goroot: String,
    /// `(dir, mtime secs)`; 0 when the directory does not exist.
    pub dirs: Vec<(PathBuf, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectCache {
    pub schema: u32,
    pub created_at: u64,
    pub fingerprint: Fingerprint,
    pub items: Vec<Detected>,
}

impl DetectCache {
    /// Usable when the fingerprint matches and it is younger than [`CACHE_MAX_AGE_SECS`].
    pub fn is_fresh(&self, fp: &Fingerprint, now: u64) -> bool {
        self.schema == CACHE_SCHEMA
            && &self.fingerprint == fp
            && now.saturating_sub(self.created_at) < CACHE_MAX_AGE_SECS
    }
}

/// Detection result as returned to the CLI and GUI.
#[derive(Debug, Clone, Serialize)]
pub struct DetectReport {
    pub items: Vec<Detected>,
    /// Served from `cache/detect.json` without scanning.
    pub from_cache: bool,
    /// The cache no longer matches the machine (only set by [`cached_only`]).
    pub stale: bool,
    pub created_at: u64,
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Fingerprint of the current machine state relevant to detection.
pub fn fingerprint(m: &Manager) -> Fingerprint {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let mut dirs: Vec<PathBuf> = m.system_path();
    let cellars = brew_cellars();
    for tool in ["node", "python", "java", "go"] {
        if let Some(h) = dirs::home_dir() {
            dirs.extend(store_scan_dirs(&h, tool, &cellars));
        }
    }
    dirs.extend(
        [
            "/Library/Java/JavaVirtualMachines",
            "/usr/lib/jvm",
            "/usr/local/go",
            "/usr/lib/go",
        ]
        .iter()
        .map(PathBuf::from),
    );
    let mut seen = std::collections::BTreeSet::new();
    let dirs = dirs
        .into_iter()
        .filter(|d| seen.insert(d.clone()))
        .map(|d| {
            let t = mtime(&d);
            (d, t)
        })
        .collect();
    Fingerprint {
        path: std::env::var("RUNA_PATH_BASE").unwrap_or_else(|_| var("PATH")),
        java_home: var("JAVA_HOME"),
        goroot: var("GOROOT"),
        dirs,
    }
}

pub fn load_cache(m: &Manager) -> Option<DetectCache> {
    crate::storage::read_json(&m.paths.detect_cache())
}

/// Forget cached results (after installs, uninstalls, adoptions ...).
pub fn invalidate_cache(m: &Manager) {
    let _ = crate::storage::remove(&m.paths.detect_cache());
}

/// Cached results if the machine did not change and the cache is < 24h old, otherwise
/// scan and save.
pub fn detect_cached(m: &Manager) -> DetectReport {
    let fp = fingerprint(m);
    if let Some(c) = load_cache(m) {
        if c.is_fresh(&fp, installer::now()) {
            return report(m, c.items, true, false, c.created_at);
        }
    }
    scan_and_save(m, fp)
}

/// Always scan (and save the cache).
pub fn detect_fresh(m: &Manager) -> DetectReport {
    scan_and_save(m, fingerprint(m))
}

/// Cached results without scanning (for instant GUI display); `stale` tells whether a
/// background re-scan is needed. `None` when there is no cache yet.
pub fn cached_only(m: &Manager) -> Option<DetectReport> {
    let c = load_cache(m)?;
    let stale = !c.is_fresh(&fingerprint(m), installer::now());
    Some(report(m, c.items, true, stale, c.created_at))
}

fn scan_and_save(m: &Manager, fp: Fingerprint) -> DetectReport {
    let items = scan(m);
    let c = DetectCache {
        schema: CACHE_SCHEMA,
        created_at: installer::now(),
        fingerprint: fp,
        items,
    };
    let _ = crate::storage::write_json(&m.paths.detect_cache(), &c);
    report(m, c.items, false, false, c.created_at)
}

fn report(
    m: &Manager,
    mut items: Vec<Detected>,
    cached: bool,
    stale: bool,
    at: u64,
) -> DetectReport {
    annotate(m, &mut items);
    DetectReport {
        items,
        from_cache: cached,
        stale,
        created_at: at,
    }
}

/// Fill `adopted` / `adopted_as` / `disabled` from the current receipts.
pub fn annotate(m: &Manager, items: &mut [Detected]) {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    for d in items.iter_mut() {
        d.adopted = false;
        d.adopted_as = None;
        d.disabled = false;
        let path_dir = d.exe.parent().map(Path::to_path_buf);
        for v in m.installed(&d.tool) {
            let Some(r) = m.receipt(&d.tool, &v) else {
                continue;
            };
            if !r.is_adopted() {
                continue;
            }
            let hit = r
                .home
                .as_deref()
                .map(|h| canon(h) == d.home)
                .unwrap_or(false)
                || r.origin.as_deref().is_some_and(|o| {
                    canon(o) == d.home || o == d.found_in || Some(o) == path_dir.as_deref()
                });
            if hit {
                d.adopted = true;
                d.adopted_as = Some(v.clone());
                d.disabled = r.disabled;
                break;
            }
        }
    }
}

// ------------------------------------------------------------------ adopt

/// Adopt a detected runtime by writing a registry receipt pointing at its real home.
/// No links are created and the original installation is never touched.
pub fn adopt(m: &Manager, d: &Detected, name: Option<&str>) -> Result<String> {
    m.plugin(&d.tool)?;
    let version = name.unwrap_or(&d.version).to_string();
    installer::validate_version_name(&version)?;
    if std::fs::symlink_metadata(m.paths.install_dir(&d.tool, &version)).is_ok()
        || m.receipt(&d.tool, &version).is_some()
    {
        bail!(
            "{} {} 已存在，可用 --name 指定其它名称（如 {}-system）",
            d.tool,
            version,
            version
        );
    }
    let home = std::fs::canonicalize(&d.home)
        .with_context(|| format!("安装目录不存在: {}", d.home.display()))?;
    for v in m.installed(&d.tool) {
        if let Some(h) = m.receipt(&d.tool, &v).and_then(|r| r.home) {
            if std::fs::canonicalize(&h).unwrap_or(h) == home {
                bail!("{} 已被接管为 {} {v}", home.display(), d.tool);
            }
        }
    }
    let exe_name = d
        .exe
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.trim_end_matches(".exe").to_string())
        .unwrap_or_else(|| d.tool.clone());
    let bin = real_bin_dir(&home, &exe_name)
        .ok_or_else(|| anyhow!("在 {} 中找不到 {exe_name}", home.display()))?;
    let (provider, formula) = classify(&home);
    let receipt = Receipt {
        tool: d.tool.clone(),
        version: version.clone(),
        source: "adopted".into(),
        installed_at: installer::now(),
        origin: Some(home.clone()),
        home: Some(home),
        bin_dirs: vec![bin],
        provider: Some(provider),
        formula,
        ..Default::default()
    };
    installer::write_receipt(&m.paths, &receipt).context("写入安装记录失败")?;
    Ok(version)
}

// ------------------------------------------------------------------ migration

/// What [`migrate`] did (or would do) with one legacy adoption.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationAction {
    pub tool: String,
    pub version: String,
    /// `convert` (symlink adoption -> registry receipt) or `drop` (cannot be resolved).
    pub action: String,
    pub old_origin: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub provider: Option<String>,
    /// Links / empty dirs inside the runa home that are removed (never followed).
    pub removes: Vec<PathBuf>,
    pub note: String,
}

/// Convert legacy symlink adoptions into registry receipts.
///
/// Only links inside `installs/` are removed (never followed); the receipt is rewritten
/// to point at the real home. Records whose real home cannot be found (e.g. the old
/// `/usr` stub adoption when the stub has no JDK behind it) are dropped.
pub fn migrate(m: &Manager, dry_run: bool) -> Result<Vec<MigrationAction>> {
    let mut out = Vec::new();
    let mut taken: Vec<PathBuf> = Vec::new();
    // homes of receipts that are already registry based
    for tool in m.registry.names() {
        for v in m.installed(&tool) {
            if let Some(r) = m.receipt(&tool, &v) {
                let legacy = std::fs::symlink_metadata(m.paths.install_dir(&tool, &v)).is_ok();
                if let (false, Some(h)) = (legacy, r.home) {
                    taken.push(h);
                }
            }
        }
    }
    for tool in m.registry.names() {
        let Ok(plugin) = m.plugin(&tool) else {
            continue;
        };
        for v in m.installed(&tool) {
            let Some(r) = m.receipt(&tool, &v) else {
                continue;
            };
            if !r.is_adopted() {
                continue;
            }
            let dir = m.paths.install_dir(&tool, &v);
            let legacy = std::fs::symlink_metadata(&dir).is_ok();
            let bad_origin = r.origin.as_deref().map(is_system_prefix).unwrap_or(false);
            if !legacy && !bad_origin && r.home.is_some() {
                continue;
            }
            let removes = artifacts(&dir);
            let resolved = resolve_legacy(m, &plugin, &v, &r);
            let mut act = MigrationAction {
                tool: tool.clone(),
                version: v.clone(),
                action: "drop".into(),
                old_origin: r.origin.clone(),
                home: None,
                provider: None,
                removes,
                note: String::new(),
            };
            match resolved {
                Some((home, _)) if taken.contains(&home) => {
                    act.note =
                        format!("真实位置 {} 已被其它版本接管，删除重复记录", home.display());
                    act.home = Some(home);
                }
                Some((home, bin)) => {
                    let (provider, formula) = classify(&home);
                    act.action = "convert".into();
                    act.note = if bad_origin {
                        format!(
                            "旧记录指向 {}，已重新定位到真实安装",
                            r.origin.as_deref().unwrap_or(Path::new("?")).display()
                        )
                    } else {
                        "符号链接接管转换为登记接管".into()
                    };
                    act.home = Some(home.clone());
                    act.provider = Some(provider.clone());
                    taken.push(home.clone());
                    if !dry_run {
                        remove_artifacts(&dir)?;
                        let new = Receipt {
                            origin: Some(home.clone()),
                            home: Some(home.clone()),
                            bin_dirs: vec![bin],
                            provider: Some(provider),
                            formula,
                            ..r.clone()
                        };
                        installer::write_receipt(&m.paths, &new)?;
                        m.clear_current_link_to(&tool, std::slice::from_ref(&dir));
                    }
                }
                None => {
                    act.note =
                        "无法定位真实安装位置（可能只是系统存根或已被删除），删除该记录".into();
                }
            }
            if act.action == "drop" && !dry_run {
                remove_artifacts(&dir)?;
                let _ = std::fs::remove_file(installer::receipt_path(&m.paths, &tool, &v));
                m.clear_current_link_to(&tool, std::slice::from_ref(&dir));
            }
            out.push(act);
        }
    }
    Ok(out)
}

/// Real `(home, bin dir)` behind a legacy (symlink / origin-only) adoption record. The
/// recorded paths are never trusted as a home by themselves: the runtime is asked.
pub(crate) fn resolve_legacy(
    m: &Manager,
    plugin: &crate::plugins::PluginRef,
    version: &str,
    r: &Receipt,
) -> Option<(PathBuf, PathBuf)> {
    let tool = plugin.name().to_string();
    let dir = m.paths.install_dir(&tool, version);
    for exe in plugin.detect_exes() {
        let Some(p) = legacy_exe(&dir, r.origin.as_deref(), r.home.as_deref(), exe, plugin) else {
            continue;
        };
        let real = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if let Some(home) = query_home(&tool, &p).or_else(|| guess_home(&real, exe)) {
            if let Some(bin) = real_bin_dir(&home, exe) {
                return Some((home, bin));
            }
        }
    }
    None
}

/// Find the executable a legacy adoption pointed at, without trusting its layout.
fn legacy_exe(
    dir: &Path,
    origin: Option<&Path>,
    home: Option<&Path>,
    exe: &str,
    plugin: &crate::plugins::PluginRef,
) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(t) = std::fs::read_link(dir) {
        dirs.extend(plugin.bin_dirs(&t));
        dirs.push(t.join("bin"));
    }
    if let Ok(t) = std::fs::read_link(dir.join("bin").join(exe)) {
        if let Some(p) = t.parent() {
            dirs.push(p.to_path_buf());
        }
    }
    for o in [home, origin].into_iter().flatten() {
        dirs.extend(plugin.bin_dirs(o));
        dirs.push(o.join("bin"));
        dirs.push(o.to_path_buf());
    }
    crate::manager::find_exe_in(&dirs, exe)
}

/// Entries a legacy adoption created under `installs/` (links and the dirs holding them).
pub(crate) fn artifacts(dir: &Path) -> Vec<PathBuf> {
    let Ok(md) = std::fs::symlink_metadata(dir) else {
        return vec![];
    };
    if md.file_type().is_symlink() || !md.is_dir() {
        return vec![dir.to_path_buf()];
    }
    let mut v = Vec::new();
    let bin = dir.join("bin");
    for e in std::fs::read_dir(&bin).into_iter().flatten().flatten() {
        v.push(e.path());
    }
    if bin.is_dir() {
        v.push(bin);
    }
    v.push(dir.to_path_buf());
    v
}

/// Remove a legacy adoption under `installs/` without ever following a link. Fails if a
/// non-link file is found, so a real installation can never be deleted by mistake.
pub(crate) fn remove_artifacts(dir: &Path) -> Result<()> {
    let Ok(md) = std::fs::symlink_metadata(dir) else {
        return Ok(());
    };
    if md.file_type().is_symlink() || !md.is_dir() {
        return platform::remove_link(dir).with_context(|| format!("删除 {} 失败", dir.display()));
    }
    let bin = dir.join("bin");
    for e in std::fs::read_dir(&bin).into_iter().flatten().flatten() {
        let p = e.path();
        if std::fs::symlink_metadata(&p)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            std::fs::remove_file(&p)?;
        }
    }
    for d in [bin.as_path(), dir] {
        match std::fs::remove_dir(d) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(anyhow!("{} 中含有非链接文件，未删除: {e}", d.display()))
            }
            _ => {}
        }
    }
    Ok(())
}

/// Result of adopting one item in [`adopt_all`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdoptOutcome {
    pub tool: String,
    pub version: String,
    pub home: PathBuf,
    /// Name it was adopted as (`None` when it failed).
    pub adopted_as: Option<String>,
    pub error: Option<String>,
}

/// Adopt every item that is not adopted yet. When the version name is already taken
/// (e.g. the same Node version from nvm and Homebrew) it is adopted as `<version>-<provider>`.
pub fn adopt_all(m: &Manager, items: &[Detected]) -> Vec<AdoptOutcome> {
    let mut out = Vec::new();
    for d in items.iter().filter(|d| !d.adopted) {
        let taken = m.installed(&d.tool).iter().any(|v| v == &d.version);
        let name = taken.then(|| format!("{}-{}", d.version, d.provider));
        let r = adopt(m, d, name.as_deref());
        out.push(AdoptOutcome {
            tool: d.tool.clone(),
            version: d.version.clone(),
            home: d.home.clone(),
            adopted_as: r.as_ref().ok().cloned(),
            error: r.err().map(|e| format!("{e:#}")),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prefix_is_never_a_home() {
        for p in ["/", "/usr", "/usr/local", "/opt/homebrew"] {
            assert!(is_system_prefix(Path::new(p)));
        }
        assert!(!is_system_prefix(Path::new("/usr/local/go")));
    }

    #[test]
    fn guess_home_rejects_stub_in_usr_bin() {
        assert_eq!(guess_home(Path::new("/usr/bin/java"), "java"), None);
    }

    #[test]
    fn guess_home_accepts_real_layout() {
        let t = std::env::temp_dir().join(format!("runa-detect-{}", std::process::id()));
        let bin = t.join("rt").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(if cfg!(windows) { "tool.exe" } else { "tool" });
        std::fs::write(&exe, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(guess_home(&exe, "tool"), Some(t.join("rt")));
        let _ = std::fs::remove_dir_all(&t);
    }

    fn exe(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn mgr(t: &Path) -> Manager {
        Manager::with_paths(crate::Paths::with_root(t.join("runa"))).unwrap()
    }

    #[test]
    fn provider_classification() {
        let c = |p: &str| classify(Path::new(p));
        assert_eq!(
            c("/opt/homebrew/Cellar/node/26.8.1"),
            ("homebrew".into(), Some("node".into()))
        );
        assert_eq!(
            c("/usr/local/Cellar/openjdk@17/17.0.9/libexec/openjdk.jdk/Contents/Home"),
            ("homebrew".into(), Some("openjdk@17".into()))
        );
        assert_eq!(
            c("/opt/homebrew/Cellar/python@3.12/3.12.4/Frameworks/Python.framework/Versions/3.12")
                .1,
            Some("python@3.12".into())
        );
        assert_eq!(c("/Users/a/.nvm/versions/node/v24.13.0").0, "nvm");
        assert_eq!(c("/Users/a/.pyenv/versions/3.12.1").0, "pyenv");
        assert_eq!(c("/Users/a/.sdkman/candidates/java/21.0.2-tem").0, "sdkman");
        assert_eq!(c("/Users/a/.asdf/installs/golang/1.22.0/go").0, "asdf");
        assert_eq!(
            c("/Users/a/.local/share/fnm/node-versions/v22.1.0/installation").0,
            "fnm"
        );
        assert_eq!(
            c("/Users/a/Library/Java/JavaVirtualMachines/jbr-21.0.11/Contents/Home").0,
            "manual"
        );
        assert_eq!(
            c("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home").0,
            "manual"
        );
        assert_eq!(c("/usr/local/go").0, "manual");
        assert_eq!(c("/usr").0, "system");
        assert_eq!(c("/usr/lib/jvm/java-17").0, "system");
        assert_eq!(
            c("/System/Library/Frameworks/Python.framework/Versions/2.7").0,
            "system"
        );
        assert_eq!(
            c("/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9").0,
            "system"
        );
        assert_eq!(
            c("/Applications/Xcode.app/Contents/Developer/Library/Frameworks/Python3.framework/Versions/3.9").0,
            "system"
        );
    }

    #[test]
    fn store_scan_skips_symlinked_aliases() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let cand = h.join(".sdkman/candidates/java");
        std::fs::create_dir_all(cand.join("21.0.2-tem/bin")).unwrap();
        std::fs::create_dir_all(cand.join("17.0.9-tem/bin")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(cand.join("21.0.2-tem"), cand.join("current")).unwrap();
        let cellar = h.join("Cellar");
        std::fs::create_dir_all(
            cellar.join("openjdk@17/17.0.9/libexec/openjdk.jdk/Contents/Home/bin"),
        )
        .unwrap();
        std::fs::create_dir_all(cellar.join("wget/1.0/bin")).unwrap();
        std::fs::create_dir_all(
            h.join("Library/Application Support/fnm/node-versions/v22.1.0/installation/bin"),
        )
        .unwrap();
        let dirs: Vec<PathBuf> = store_bin_dirs(h, "java", std::slice::from_ref(&cellar))
            .into_iter()
            .map(|(d, _)| d)
            .collect();
        assert!(dirs.contains(&cand.join("21.0.2-tem/bin")));
        assert!(dirs.contains(&cand.join("17.0.9-tem/bin")));
        assert!(!dirs.iter().any(|d| d.starts_with(cand.join("current"))));
        assert!(
            dirs.contains(&cellar.join("openjdk@17/17.0.9/libexec/openjdk.jdk/Contents/Home/bin"))
        );
        assert!(!dirs.iter().any(|d| d.starts_with(cellar.join("wget"))));
        let node: Vec<_> = store_bin_dirs(h, "node", &[]).into_iter().collect();
        assert!(node
            .iter()
            .any(|(d, via)| via == "fnm" && d.ends_with("v22.1.0/installation/bin")));
    }

    #[test]
    fn cache_fingerprint_and_staleness() {
        let fp = Fingerprint {
            path: "/a:/b".into(),
            java_home: String::new(),
            goroot: String::new(),
            dirs: vec![(PathBuf::from("/a"), 10)],
        };
        let c = DetectCache {
            schema: CACHE_SCHEMA,
            created_at: 1_000,
            fingerprint: fp.clone(),
            items: vec![],
        };
        assert!(c.is_fresh(&fp, 1_000 + 60));
        assert!(
            !c.is_fresh(&fp, 1_000 + CACHE_MAX_AGE_SECS + 1),
            "older than 24h"
        );
        let mut moved = fp.clone();
        moved.dirs[0].1 = 11;
        assert!(!c.is_fresh(&moved, 1_001), "directory mtime changed");
        let mut env = fp.clone();
        env.java_home = "/jdk".into();
        assert!(!c.is_fresh(&env, 1_001), "JAVA_HOME changed");
        let mut path = fp;
        path.path = "/b:/a".into();
        assert!(!c.is_fresh(&path, 1_001), "PATH changed");
    }

    #[test]
    fn cache_roundtrip_in_temp_home() {
        let t = tempfile::tempdir().unwrap();
        let m = mgr(t.path());
        let fp = fingerprint(&m);
        assert_eq!(fp, fingerprint(&m), "fingerprint is stable");
        let c = DetectCache {
            schema: CACHE_SCHEMA,
            created_at: installer::now(),
            fingerprint: fp,
            items: vec![],
        };
        crate::storage::write_json(&m.paths.detect_cache(), &c).unwrap();
        let r = cached_only(&m).unwrap();
        assert!(r.from_cache && !r.stale);
        let r = detect_cached(&m);
        assert!(r.from_cache);
        invalidate_cache(&m);
        assert!(cached_only(&m).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn adopt_is_registry_only_and_disabled_is_skipped() {
        let t = tempfile::tempdir().unwrap();
        let m = mgr(t.path());
        let home = t.path().join("rt").join("node-v20.1.0");
        exe(&home.join("bin/node"), "#!/bin/sh\necho v20.1.0\n");
        let home = std::fs::canonicalize(&home).unwrap();
        let d = Detected {
            tool: "node".into(),
            version: "20.1.0".into(),
            exe: home.join("bin/node"),
            found_in: home.join("bin"),
            home: home.clone(),
            via: "PATH".into(),
            provider: "manual".into(),
            formula: None,
            adopted: false,
            adopted_as: None,
            disabled: false,
        };
        assert_eq!(adopt(&m, &d, None).unwrap(), "20.1.0");
        assert!(
            std::fs::symlink_metadata(m.paths.install_dir("node", "20.1.0")).is_err(),
            "no link"
        );
        let r = m.receipt("node", "20.1.0").unwrap();
        assert_eq!(r.home.as_deref(), Some(home.as_path()));
        assert_eq!(r.bin_dirs, vec![home.join("bin")]);
        assert_eq!(r.provider.as_deref(), Some("manual"));
        assert!(adopt(&m, &d, Some("other")).is_err(), "same home twice");
        assert_eq!(m.installed("node"), vec!["20.1.0".to_string()]);
        assert_eq!(m.match_installed("node", "20"), Some("20.1.0".into()));
        assert_eq!(m.bin_dirs("node", "20.1.0"), vec![home.join("bin")]);
        assert!(m.all_commands().contains(&"node".to_string()));
        let mut items = vec![d.clone()];
        annotate(&m, &mut items);
        assert!(items[0].adopted && items[0].adopted_as.as_deref() == Some("20.1.0"));

        // disable: still listed, but never resolved / shimmed / on PATH
        m.set_global("node", "20.1.0").unwrap();
        let w = m.set_disabled("node", "20.1.0", true, t.path()).unwrap();
        assert!(w.iter().any(|w| w.contains("全局")), "{w:?}");
        assert_eq!(m.installed("node"), vec!["20.1.0".to_string()]);
        assert!(m.enabled_versions("node").is_empty());
        assert_eq!(m.match_installed("node", "20"), None);
        assert_eq!(m.match_installed("node", "20.1.0"), None);
        assert!(!m.all_commands().contains(&"node".to_string()));
        let r = m.resolve("node", t.path()).unwrap();
        assert_eq!(r.version, None);
        let plan = m.env_plan(t.path(), &[]).unwrap();
        assert!(!plan.paths.contains(&home.join("bin")));
        annotate(&m, &mut items);
        assert!(items[0].disabled);
        m.set_disabled("node", "20.1.0", false, t.path()).unwrap();
        assert_eq!(
            m.resolve("node", t.path()).unwrap().version.as_deref(),
            Some("20.1.0")
        );
        // update_current_link points straight at the real home
        assert_eq!(
            std::fs::read_link(m.paths.current().join("node")).unwrap(),
            home
        );
    }

    #[cfg(unix)]
    #[test]
    fn migrate_symlink_adoptions() {
        let t = tempfile::tempdir().unwrap();
        let m = mgr(t.path());
        // a self-contained runtime adopted as a dir symlink (old JDK / Go style)
        let go_home = std::fs::canonicalize(t.path()).unwrap().join("go-1.22");
        exe(&go_home.join("bin/go"), "#!/bin/sh\nexit 1\n");
        let tool_dir = m.paths.tool_dir("go");
        std::fs::create_dir_all(&tool_dir).unwrap();
        std::os::unix::fs::symlink(&go_home, m.paths.install_dir("go", "1.22.0")).unwrap();
        let legacy = |tool: &str, v: &str, origin: &Path| Receipt {
            tool: tool.into(),
            version: v.into(),
            source: "adopted".into(),
            origin: Some(origin.into()),
            installed_at: 1,
            ..Default::default()
        };
        installer::write_receipt(&m.paths, &legacy("go", "1.22.0", &go_home)).unwrap();
        // a node adopted as a private bin/ of file links (old node / python style)
        let node_home = std::fs::canonicalize(t.path()).unwrap().join("node-20");
        exe(&node_home.join("bin/node"), "#!/bin/sh\nexit 1\n");
        let nd = m.paths.install_dir("node", "20.1.0");
        std::fs::create_dir_all(nd.join("bin")).unwrap();
        std::os::unix::fs::symlink(node_home.join("bin/node"), nd.join("bin/node")).unwrap();
        installer::write_receipt(&m.paths, &legacy("node", "20.1.0", &node_home.join("bin")))
            .unwrap();
        // a broken record pointing at a dir without any runtime
        let empty = t.path().join("nothing");
        std::fs::create_dir_all(&empty).unwrap();
        std::os::unix::fs::symlink(&empty, m.paths.install_dir("go", "0.0.1")).unwrap();
        installer::write_receipt(&m.paths, &legacy("go", "0.0.1", &empty)).unwrap();

        let dry = migrate(&m, true).unwrap();
        assert_eq!(dry.len(), 3);
        assert!(
            std::fs::symlink_metadata(m.paths.install_dir("go", "1.22.0")).is_ok(),
            "dry run changes nothing"
        );
        let acts = migrate(&m, false).unwrap();
        let find = |t: &str, v: &str| acts.iter().find(|a| a.tool == t && a.version == v).unwrap();
        assert_eq!(find("go", "1.22.0").action, "convert");
        assert_eq!(find("node", "20.1.0").action, "convert");
        assert_eq!(find("go", "0.0.1").action, "drop");
        // links gone, originals untouched, receipts rewritten
        assert!(std::fs::symlink_metadata(m.paths.install_dir("go", "1.22.0")).is_err());
        assert!(std::fs::symlink_metadata(&nd).is_err());
        assert!(go_home.join("bin/go").is_file() && node_home.join("bin/node").is_file());
        assert!(empty.is_dir());
        let r = m.receipt("go", "1.22.0").unwrap();
        assert_eq!(r.home.as_deref(), Some(go_home.as_path()));
        assert_eq!(r.bin_dirs, vec![go_home.join("bin")]);
        assert_eq!(r.installed_at, 1);
        assert_eq!(
            m.receipt("node", "20.1.0").unwrap().home.as_deref(),
            Some(node_home.as_path())
        );
        assert!(m.receipt("go", "0.0.1").is_none());
        assert_eq!(m.installed("go"), vec!["1.22.0".to_string()]);
        assert!(migrate(&m, false).unwrap().is_empty(), "idempotent");
    }

    #[cfg(unix)]
    #[test]
    fn remove_artifacts_refuses_real_files() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("v");
        std::fs::create_dir_all(d.join("bin")).unwrap();
        std::fs::write(d.join("bin/real"), "x").unwrap();
        assert!(remove_artifacts(&d).is_err());
        assert!(d.join("bin/real").exists());
    }
}
