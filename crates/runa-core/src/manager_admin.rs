//! Inspect, upgrade and uninstall version managers (the counterpart of [`crate::bootstrap`]).
//!
//! [`plan`] never changes anything (dry run for the CLI and the GUI confirmation);
//! [`execute`] carries a plan out. Uninstalling follows the way the manager was installed
//! (brew / winget / scoop command, or removing its own directory for script / git
//! installs), removes the shell rc lines its installer added (shown in the plan, files
//! backed up first) and unregisters the runtimes runa had registered through it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::bootstrap::{self, NVM_VERSION};
use crate::managers::{self, ManagerCmd, Probe};
use crate::platform;
use crate::Manager;

/// Receipt provider names belonging to a manager.
fn providers_of(manager: &str) -> &'static [&'static str] {
    match manager {
        "nvm" => &["nvm"],
        "nvm-windows" => &["nvm-windows"],
        "fnm" => &["fnm"],
        "pyenv" => &["pyenv"],
        "pyenv-win" => &["pyenv-win"],
        "sdkman" => &["sdkman"],
        "asdf" => &["asdf"],
        "mise" => &["mise"],
        _ => &[],
    }
}

/// Package ids per install origin.
fn package_id(manager: &str, origin: &str) -> Option<&'static str> {
    Some(match (manager, origin) {
        ("nvm" | "fnm" | "pyenv" | "asdf" | "mise", "brew") => match manager {
            "nvm" => "nvm",
            "fnm" => "fnm",
            "pyenv" => "pyenv",
            "asdf" => "asdf",
            _ => "mise",
        },
        ("nvm-windows", "winget") => "CoreyButler.NVMforWindows",
        ("fnm", "winget") => "Schniz.fnm",
        ("mise", "winget") => "jdx.mise",
        ("nvm-windows", "scoop") => "nvm",
        ("fnm", "scoop") => "fnm",
        ("pyenv-win", "scoop") => "pyenv",
        ("mise", "scoop") => "mise",
        _ => return None,
    })
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RegisteredRuntime {
    pub tool: String,
    pub version: String,
    /// It is runa's global version for its tool.
    pub global: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagerInfo {
    pub name: String,
    pub tools: Vec<String>,
    pub supported: bool,
    pub installed: bool,
    pub version: Option<String>,
    /// Executable / init script.
    pub entry: Option<PathBuf>,
    /// The manager's own directory (NVM_DIR, SDKMAN_DIR, PYENV_ROOT, data dir ...).
    pub data_dir: Option<PathBuf>,
    /// `brew`, `winget`, `scoop`, `script`, `git`, `system` or `unknown`.
    pub origin: String,
    /// Runtimes runa registered through it.
    pub runtimes: Vec<RegisteredRuntime>,
    /// How it would be installed (when not installed).
    pub install_method: Option<bootstrap::Method>,
}

/// Executable / init script and data dir of an installed manager.
fn locate_any(probe: &Probe, manager: &str) -> Option<(PathBuf, Option<PathBuf>)> {
    if manager == "asdf" {
        let dirs = probe.env.manager_dir("asdf");
        let bins: Vec<PathBuf> = dirs.iter().map(|d| d.join("bin")).collect();
        let exe = probe.find("asdf", &bins)?;
        return Some((exe, dirs.into_iter().find(|d| d.is_dir())));
    }
    let tool = bootstrap::tools_of(manager).first()?;
    let l = probe.locate(manager, tool)?;
    Some((l.entry, l.store.is_dir().then_some(l.store)))
}

/// How a manager was installed, from where its executable lives.
pub fn origin_of(manager: &str, entry: &Path, data_dir: Option<&Path>) -> String {
    let real = platform::canonicalize(entry);
    let k = platform::path_key_for(&real, true);
    let raw = platform::path_key_for(entry, true);
    let o = if k.contains("/cellar/")
        || raw.starts_with("/opt/homebrew/")
        || raw.contains("/.linuxbrew/")
    {
        "brew"
    } else if k.contains("/scoop/") {
        "scoop"
    } else if k.contains("/microsoft/winget/") || k.contains("/program files/") {
        "winget"
    } else if crate::detect::classify(real.parent().unwrap_or(&real)).0 == "system" {
        "system"
    } else {
        match manager {
            "nvm" | "sdkman" | "fnm" | "mise" => "script",
            "pyenv" | "asdf" => {
                if data_dir.is_some_and(|d| d.join(".git").is_dir()) {
                    if manager == "asdf" {
                        "git"
                    } else {
                        "script"
                    }
                } else {
                    "unknown"
                }
            }
            "pyenv-win" => "script",
            _ => "unknown",
        }
    };
    o.into()
}

fn run_capture(program: &Path, args: &[&str], env: &[(&str, String)]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k.to_string(), v.clone())))
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let s =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

/// First version-looking token (`0.40.3`, `v1.2.3`, `2024.1.0`).
fn version_token(s: &str) -> Option<String> {
    s.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .map(|t| t.trim_start_matches('v'))
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(str::to_string)
}

fn version_of(manager: &str, entry: &Path, data_dir: Option<&Path>) -> Option<String> {
    match manager {
        "nvm" => {
            let d = data_dir?.display().to_string();
            let out = run_capture(
                Path::new("bash"),
                &[
                    "-c",
                    r#"export NVM_DIR="$1"; . "$NVM_DIR/nvm.sh" --no-use && nvm --version"#,
                    "x",
                    &d,
                ],
                &[],
            )?;
            version_token(&out)
        }
        "sdkman" => std::fs::read_to_string(data_dir?.join("var").join("version"))
            .ok()
            .and_then(|s| version_token(&s).or_else(|| Some(s.trim().to_string())))
            .filter(|s| !s.is_empty()),
        _ => version_token(&run_capture(entry, &["--version"], &[])?),
    }
}

/// Runtimes runa registered with `manager` as provider.
pub fn registered_runtimes(m: &Manager, manager: &str) -> Vec<RegisteredRuntime> {
    let provs = providers_of(manager);
    let mut out = Vec::new();
    for tool in m.registry.names() {
        let global = m.global_version(&tool);
        for v in m.installed(&tool) {
            let Some(r) = m.receipt(&tool, &v) else {
                continue;
            };
            if r.is_adopted() && r.provider.as_deref().is_some_and(|p| provs.contains(&p)) {
                out.push(RegisteredRuntime {
                    tool: tool.clone(),
                    global: global.as_deref() == Some(v.as_str()),
                    version: v,
                });
            }
        }
    }
    out
}

pub fn info(m: &Manager, probe: &Probe) -> Vec<ManagerInfo> {
    bootstrap::MANAGERS
        .iter()
        .filter(|n| bootstrap::supported_on(n, cfg!(windows)))
        .map(|n| info_one(m, probe, n))
        .collect()
}

pub fn info_one(m: &Manager, probe: &Probe, name: &str) -> ManagerInfo {
    let found = locate_any(probe, name);
    let (entry, data_dir) = match &found {
        Some((e, d)) => (Some(e.clone()), d.clone()),
        None => (None, None),
    };
    ManagerInfo {
        name: name.into(),
        tools: bootstrap::tools_of(name)
            .iter()
            .map(|t| t.to_string())
            .collect(),
        supported: bootstrap::supported_on(name, cfg!(windows)),
        installed: found.is_some(),
        version: entry
            .as_deref()
            .and_then(|e| version_of(name, e, data_dir.as_deref())),
        origin: entry
            .as_deref()
            .map(|e| origin_of(name, e, data_dir.as_deref()))
            .unwrap_or_else(|| "-".into()),
        entry,
        data_dir,
        runtimes: registered_runtimes(m, name),
        install_method: if found.is_some() {
            None
        } else {
            bootstrap::methods(probe, name).into_iter().next()
        },
    }
}

// ------------------------------------------------------------------ plans

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RcEdit {
    pub file: PathBuf,
    /// `(1-based line number, text)` to remove.
    pub lines: Vec<(usize, String)>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    Run,
    Refuse,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManagerPlan {
    pub manager: String,
    /// `upgrade` or `uninstall`.
    pub action: String,
    pub kind: PlanKind,
    pub origin: String,
    /// Command to run first (package-manager uninstall / upgrade command).
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub command_line: Option<String>,
    /// Directories deleted (manager dir with every runtime it installed).
    pub remove_dirs: Vec<PathBuf>,
    /// Shell rc lines removed (files are backed up as `<file>.runa-bak`).
    pub rc_edits: Vec<RcEdit>,
    /// Runtimes that disappear and get unregistered from runa.
    pub runtimes: Vec<RegisteredRuntime>,
    /// Runtime versions installed in the manager's data dir (e.g. `~/.nvm/versions/node/*`).
    /// When non-empty the user chooses between keeping them (`keep_data`) or deleting them.
    pub data_runtimes: Vec<PathBuf>,
    /// Uninstall keeps the data dir: runtimes stay registered, re-labelled `manual`.
    pub keep_data: bool,
    pub summary: String,
}

/// Sub-directories of a manager's data dir that hold installed runtimes, and how deep
/// the version directories sit below them.
fn runtime_subdirs(manager: &str) -> &'static [(&'static str, usize)] {
    match manager {
        "nvm" => &[("versions", 2), ("alias", 0)],
        "pyenv" | "pyenv-win" => &[("versions", 1)],
        "sdkman" => &[("candidates", 2)],
        "asdf" | "mise" => &[("installs", 2)],
        "fnm" => &[("node-versions", 1), ("aliases", 0)],
        _ => &[],
    }
}

fn dirs_at(d: &Path, depth: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else {
        return vec![];
    };
    let mut kids: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n != "current" && !n.to_string_lossy().starts_with('.'))
        })
        .collect();
    kids.sort();
    if depth <= 1 {
        return kids;
    }
    kids.iter().flat_map(|k| dirs_at(k, depth - 1)).collect()
}

/// Runtime version directories inside `data` (empty when there are none).
pub fn data_runtimes(manager: &str, data: &Path) -> Vec<PathBuf> {
    if manager == "nvm-windows" {
        return dirs_at(data, 1)
            .into_iter()
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with('v') && n[1..].starts_with(|c: char| c.is_ascii_digit())
                })
            })
            .collect();
    }
    runtime_subdirs(manager)
        .iter()
        .filter(|(_, depth)| *depth > 0)
        .flat_map(|(sub, depth)| dirs_at(&data.join(sub), *depth))
        .collect()
}

/// Shell startup files installers write to.
pub fn rc_files(home: &Path) -> Vec<PathBuf> {
    [
        ".bashrc",
        ".bash_profile",
        ".profile",
        ".zshrc",
        ".zprofile",
        ".zshenv",
        ".config/fish/config.fish",
    ]
    .iter()
    .map(|f| f.split('/').fold(home.to_path_buf(), |p, s| p.join(s)))
    .collect()
}

/// Whether an rc line belongs to `manager`'s installer (never runa's own block).
pub fn rc_line_matches(manager: &str, line: &str) -> bool {
    let l = line.trim();
    if l.is_empty() || l.contains("runa") {
        return false;
    }
    let any = |ns: &[&str]| ns.iter().any(|n| l.contains(n));
    match manager {
        "nvm" => any(&["NVM_DIR", "nvm.sh", "nvm/bash_completion"]),
        "sdkman" => any(&["SDKMAN", "sdkman-init.sh"]),
        "pyenv" => any(&[
            "PYENV_ROOT",
            "pyenv init",
            "pyenv virtualenv-init",
            ".pyenv/bin",
        ]),
        "fnm" => any(&["fnm env", "/fnm\"", "/fnm:", "FNM_PATH", "fnm completions"]),
        "mise" => any(&["mise activate", "mise.run"]),
        "asdf" => any(&["asdf.sh", "asdf.fish", "ASDF_DATA_DIR", "asdf/completions"]),
        _ => false,
    }
}

pub fn rc_edits_for(manager: &str, home: &Path) -> Vec<RcEdit> {
    let mut out = Vec::new();
    for f in rc_files(home) {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let lines: Vec<(usize, String)> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| rc_line_matches(manager, l))
            .map(|(i, l)| (i + 1, l.to_string()))
            .collect();
        if !lines.is_empty() {
            out.push(RcEdit { file: f, lines });
        }
    }
    out
}

/// A dir runa may delete for a manager: inside the user's home (or app data), not the
/// home itself, not a shared prefix.
fn safe_dir(probe: &Probe, d: &Path) -> bool {
    let env = &probe.env;
    let within = |r: &Path| platform::path_within(d, r) && !platform::same_path(d, r);
    !crate::locations::is_system_prefix(d, env)
        && d.components().count() >= 3
        && (within(&env.home)
            || env.appdata.as_deref().is_some_and(within)
            || env.local_appdata.as_deref().is_some_and(within))
}

fn sh_cmd(script: &str, args: &[String]) -> (Vec<String>, String) {
    let mut v = vec![
        "bash".to_string(),
        "-c".into(),
        script.to_string(),
        "runa".into(),
    ];
    v.extend(args.iter().cloned());
    (v, script.to_string())
}

/// What upgrading / uninstalling `name` would do. Changes nothing.
pub fn plan(m: &Manager, probe: &Probe, name: &str, action: &str) -> Result<ManagerPlan> {
    plan_with(m, probe, name, action, false)
}

/// Like [`plan`]; with `keep_data` an uninstall removes only the manager itself (package /
/// scripts and rc lines) and keeps the runtimes it installed.
pub fn plan_with(
    m: &Manager,
    probe: &Probe,
    name: &str,
    action: &str,
    keep_data: bool,
) -> Result<ManagerPlan> {
    if !bootstrap::MANAGERS.contains(&name) {
        bail!("未知的版本管理器: {name}");
    }
    if action != "upgrade" && action != "uninstall" {
        bail!("未知操作: {action}");
    }
    let info = info_one(m, probe, name);
    if !info.installed {
        bail!("{name} 未安装");
    }
    let entry = info.entry.clone().unwrap_or_default();
    let data = info.data_dir.clone();
    let mut p = ManagerPlan {
        manager: name.into(),
        action: action.into(),
        kind: PlanKind::Run,
        origin: info.origin.clone(),
        command: None,
        env: vec![],
        command_line: None,
        remove_dirs: vec![],
        data_runtimes: vec![],
        keep_data: keep_data && action == "uninstall",
        rc_edits: vec![],
        runtimes: if action == "uninstall" {
            info.runtimes.clone()
        } else {
            vec![]
        },
        summary: String::new(),
    };
    let refuse = |mut p: ManagerPlan, why: String| {
        p.kind = PlanKind::Refuse;
        p.summary = why;
        p
    };
    let tool = |n: &str| probe.find(n, &[]).map(|x| x.display().to_string());
    let s = |x: &str| x.to_string();
    let origin = info.origin.as_str();
    if action == "uninstall" {
        p.data_runtimes = data
            .as_deref()
            .map(|d| data_runtimes(name, d))
            .unwrap_or_default();
        if p.keep_data && p.data_runtimes.is_empty() && p.runtimes.is_empty() {
            p.keep_data = false;
        }
    }
    let pkg = package_id(name, origin);
    match (action, origin) {
        (_, "system" | "unknown" | "-") => {
            return Ok(refuse(
                p,
                format!(
                    "无法确定 {name} 的安装方式（{}），runa 不会{}它；请按其文档手动处理",
                    entry.display(),
                    if action == "upgrade" {
                        "升级"
                    } else {
                        "卸载"
                    }
                ),
            ))
        }
        ("upgrade", "brew") => {
            let pkg = pkg.context("未知的 Homebrew formula")?;
            p.command = Some(vec![
                tool("brew").unwrap_or(s("brew")),
                s("upgrade"),
                s(pkg),
            ]);
            p.command_line = Some(format!("brew upgrade {pkg}"));
        }
        ("uninstall", "brew") => {
            let pkg = pkg.context("未知的 Homebrew formula")?;
            p.command = Some(vec![
                tool("brew").unwrap_or(s("brew")),
                s("uninstall"),
                s(pkg),
            ]);
            p.command_line = Some(format!("brew uninstall {pkg}"));
        }
        ("upgrade", "winget") => {
            let pkg = pkg.context("未知的 winget id")?;
            p.command = Some(vec![
                tool("winget").unwrap_or(s("winget")),
                s("upgrade"),
                s("--id"),
                s(pkg),
                s("--exact"),
            ]);
            p.command_line = Some(format!("winget upgrade --id {pkg} --exact"));
        }
        ("uninstall", "winget") => {
            let pkg = pkg.context("未知的 winget id")?;
            p.command = Some(vec![
                tool("winget").unwrap_or(s("winget")),
                s("uninstall"),
                s("--id"),
                s(pkg),
                s("--exact"),
            ]);
            p.command_line = Some(format!("winget uninstall --id {pkg} --exact"));
        }
        ("upgrade", "scoop") => {
            let pkg = pkg.context("未知的 scoop 应用")?;
            p.command = Some(vec![
                tool("scoop").unwrap_or(s("scoop")),
                s("update"),
                s(pkg),
            ]);
            p.command_line = Some(format!("scoop update {pkg}"));
        }
        ("uninstall", "scoop") => {
            let pkg = pkg.context("未知的 scoop 应用")?;
            p.command = Some(vec![
                tool("scoop").unwrap_or(s("scoop")),
                s("uninstall"),
                s(pkg),
            ]);
            p.command_line = Some(format!("scoop uninstall {pkg}"));
        }
        ("upgrade", _) => {
            let d = data
                .clone()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            let (cmd, line) = match name {
                "nvm" => sh_cmd(
                    &format!("export NVM_DIR=\"$1\"; curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/{NVM_VERSION}/install.sh | PROFILE=/dev/null bash"),
                    &[d],
                ),
                "sdkman" => sh_cmd(
                    r#"export SDKMAN_DIR="$1" sdkman_auto_answer=true; . "$SDKMAN_DIR/bin/sdkman-init.sh" && sdk selfupdate"#,
                    &[d],
                ),
                "mise" => (vec![entry.display().to_string(), s("self-update"), s("--yes")], s("mise self-update --yes")),
                "fnm" => sh_cmd(
                    "curl -fsSL https://fnm.vercel.app/install | bash -s -- --skip-shell --install-dir \"$1\"",
                    &[d],
                ),
                "pyenv" | "asdf" | "pyenv-win" => (
                    vec![tool("git").unwrap_or(s("git")), s("-C"), d.clone(), s("pull"), s("--ff-only")],
                    format!("git -C {d} pull --ff-only"),
                ),
                _ => return Ok(refuse(p, format!("不支持升级 {name}"))),
            };
            p.command = Some(cmd);
            p.command_line = Some(line);
        }
        ("uninstall", _) => {
            // script / git installs live in their own directory
            let Some(d) = data.clone() else {
                return Ok(refuse(p, format!("找不到 {name} 的安装目录")));
            };
            if p.keep_data {
                // delete the manager's own files, keep the directories holding runtimes
                let keep: Vec<&str> = runtime_subdirs(name).iter().map(|(s, _)| *s).collect();
                if keep.is_empty() {
                    return Ok(refuse(
                        p,
                        format!("{name} 的运行时与程序在同一目录中，无法只卸载管理器"),
                    ));
                }
                let mut kids: Vec<PathBuf> = std::fs::read_dir(&d)
                    .map(|rd| rd.flatten().map(|e| e.path()).collect())
                    .unwrap_or_default();
                kids.sort();
                p.remove_dirs.extend(kids.into_iter().filter(|k| {
                    !k.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| keep.contains(&n))
                }));
            } else {
                p.remove_dirs.push(d);
            }
            if name == "mise" {
                p.remove_dirs.push(entry.clone());
            }
            p.rc_edits = rc_edits_for(name, &probe.env.home);
        }
        _ => unreachable!(),
    }
    if action == "uninstall" {
        // package-manager installs: the runtimes still live in the data dir
        if p.command.is_some() {
            if let Some(d) = data.clone().filter(|d| d.is_dir() && !p.keep_data) {
                p.remove_dirs.push(d);
            }
            p.rc_edits = rc_edits_for(name, &probe.env.home);
        }
        for d in &p.remove_dirs {
            if !safe_dir(probe, d) {
                return Ok(refuse(
                    p.clone(),
                    format!("拒绝删除 {}：不在用户目录内或属于共享目录", d.display()),
                ));
            }
        }
    }
    p.summary = match action {
        "upgrade" => format!(
            "升级 {name}：{}",
            p.command_line.clone().unwrap_or_default()
        ),
        _ => {
            let mut parts = vec![];
            if let Some(c) = &p.command_line {
                parts.push(c.clone());
            }
            for d in &p.remove_dirs {
                parts.push(format!("删除 {}", d.display()));
            }
            if !p.rc_edits.is_empty() {
                parts.push(format!(
                    "从 {} 个 shell 配置文件中移除 {name} 的加载代码",
                    p.rc_edits.len()
                ));
            }
            if p.keep_data {
                parts.push(format!(
                    "保留 {} 个已安装的版本{}",
                    p.data_runtimes.len(),
                    if p.runtimes.is_empty() {
                        String::new()
                    } else {
                        format!("，{} 个已登记的运行时改为“手动安装”", p.runtimes.len())
                    }
                ));
            } else if !p.runtimes.is_empty() {
                parts.push(format!(
                    "移除 {} 个由它管理的运行时并取消登记",
                    p.runtimes.len()
                ));
            }
            format!("卸载 {name}：{}", parts.join("；"))
        }
    };
    Ok(p)
}

/// What [`execute`] did besides the plan itself.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ExecReport {
    pub unregistered: Vec<RegisteredRuntime>,
    pub notices: Vec<String>,
}

fn remove_rc_lines(e: &RcEdit) -> Result<()> {
    let text = std::fs::read_to_string(&e.file)?;
    let mut bak = e.file.clone().into_os_string();
    bak.push(".runa-bak");
    std::fs::write(PathBuf::from(bak), &text)?;
    let drop: std::collections::BTreeSet<usize> = e
        .lines
        .iter()
        .filter(|(n, l)| text.lines().nth(n - 1) == Some(l.as_str()))
        .map(|(n, _)| *n)
        .collect();
    let mut out: String = text
        .lines()
        .enumerate()
        .filter(|(i, _)| !drop.contains(&(i + 1)))
        .map(|(_, l)| format!("{l}\n"))
        .collect();
    if !text.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    std::fs::write(&e.file, out)?;
    Ok(())
}

/// Carry out a plan from [`plan`], streaming command output to `on_line`.
pub fn execute(
    m: &Manager,
    probe: &Probe,
    p: &ManagerPlan,
    on_line: &mut dyn FnMut(String),
) -> Result<ExecReport> {
    if p.kind == PlanKind::Refuse {
        bail!("{}", p.summary);
    }
    let mut rep = ExecReport::default();
    if let Some(argv) = &p.command {
        let cmd = ManagerCmd {
            program: PathBuf::from(&argv[0]),
            args: argv[1..].to_vec(),
            env: p.env.clone(),
            display: p.command_line.clone().unwrap_or_default(),
        };
        managers::run_streaming(&cmd, on_line)?;
    }
    for d in &p.remove_dirs {
        if !safe_dir(probe, d) {
            bail!("拒绝删除 {}", d.display());
        }
        on_line(format!("删除 {}", d.display()));
        match std::fs::symlink_metadata(d) {
            Ok(md) if md.is_dir() => {
                std::fs::remove_dir_all(d).with_context(|| format!("删除 {} 失败", d.display()))?
            }
            Ok(_) => {
                std::fs::remove_file(d).with_context(|| format!("删除 {} 失败", d.display()))?
            }
            Err(_) => {}
        }
    }
    for e in &p.rc_edits {
        on_line(format!("清理 {}（已备份为 .runa-bak）", e.file.display()));
        remove_rc_lines(e).with_context(|| format!("修改 {} 失败", e.file.display()))?;
    }
    if p.action == "uninstall" && p.keep_data {
        for r in &p.runtimes {
            if let Some(mut rc) = m.receipt(&r.tool, &r.version) {
                rc.provider = Some("manual".into());
                crate::installer::write_receipt(&m.paths, &rc)?;
            }
        }
        if !p.runtimes.is_empty() {
            rep.notices.push(format!(
                "已保留 {} 个运行时，来源改为“手动安装”",
                p.runtimes.len()
            ));
        }
        crate::detect::invalidate_cache(m);
    } else if p.action == "uninstall" {
        for r in &p.runtimes {
            if let Ok(pl) =
                crate::uninstall::plan(m, &r.tool, &r.version, crate::uninstall::Mode::Forget)
            {
                crate::uninstall::execute(m, &pl)?;
            }
            if r.global {
                m.unset_global(&r.tool)?;
                rep.notices.push(format!(
                    "{} {} 曾是 runa 的全局版本，已清除 {} 的全局设置",
                    r.tool, r.version, r.tool
                ));
            }
            rep.unregistered.push(r.clone());
        }
        crate::detect::invalidate_cache(m);
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{write_receipt, Receipt};
    use crate::locations::Env;

    fn probe(home: &Path, path: Vec<PathBuf>) -> Probe {
        Probe {
            env: Env {
                home: home.to_path_buf(),
                ..Default::default()
            },
            path,
            fallback: vec![],
        }
    }

    fn adopt(m: &Manager, tool: &str, v: &str, home: &Path, provider: &str) {
        write_receipt(
            &m.paths,
            &Receipt {
                tool: tool.into(),
                version: v.into(),
                source: "adopted".into(),
                home: Some(home.into()),
                origin: Some(home.into()),
                bin_dirs: vec![home.join("bin")],
                provider: Some(provider.into()),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn rc_line_matching() {
        assert!(rc_line_matches("nvm", r#"export NVM_DIR="$HOME/.nvm""#));
        assert!(rc_line_matches(
            "nvm",
            r#"[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh""#
        ));
        assert!(rc_line_matches(
            "sdkman",
            r#"[[ -s "$HOME/.sdkman/bin/sdkman-init.sh" ]] && source "$HOME/.sdkman/bin/sdkman-init.sh""#
        ));
        assert!(rc_line_matches(
            "sdkman",
            "#THIS MUST BE AT THE END OF THE FILE FOR SDKMAN TO WORK!!!"
        ));
        assert!(
            !rc_line_matches("nvm", "# >>> runa >>> NVM_DIR"),
            "runa's own block is never touched"
        );
        assert!(!rc_line_matches("nvm", "alias ll='ls -l'"));
        assert_eq!(version_token("nvm 0.40.3"), Some("0.40.3".into()));
        assert_eq!(version_token("fnm v1.37.1"), Some("1.37.1".into()));
    }

    #[test]
    fn origin_detection() {
        assert_eq!(
            origin_of("fnm", Path::new("/opt/homebrew/bin/fnm"), None),
            "brew"
        );
        assert_eq!(
            origin_of("mise", Path::new(r"C:\Users\a\scoop\shims\mise.exe"), None),
            "scoop"
        );
        assert_eq!(
            origin_of(
                "fnm",
                Path::new(r"C:\Users\a\AppData\Local\Microsoft\WinGet\Links\fnm.exe"),
                None
            ),
            "winget"
        );
        assert_eq!(
            origin_of("pyenv", Path::new("/usr/bin/pyenv"), None),
            "system"
        );
        assert_eq!(
            origin_of("nvm", Path::new("/h/.nvm/nvm.sh"), None),
            "script"
        );
        assert_eq!(
            origin_of("pyenv", Path::new("/h/.pyenv/bin/pyenv"), None),
            "unknown"
        );
    }

    /// Script-installed nvm: plan lists dir, rc lines and runtimes; execute removes them
    /// (rc backed up), unregisters the runtimes and clears runa's global.
    #[cfg(unix)]
    #[test]
    fn uninstall_script_nvm_with_runtimes() {
        let t = tempfile::tempdir().unwrap();
        let h = platform::canonicalize(t.path());
        let nvm = h.join(".nvm");
        std::fs::create_dir_all(nvm.join("versions/node/v20.1.0/bin")).unwrap();
        std::fs::write(nvm.join("nvm.sh"), "nvm() { echo 0.40.3; }\n").unwrap();
        let rc = h.join(".zshrc");
        std::fs::write(&rc, "alias ll='ls -l'\nexport NVM_DIR=\"$HOME/.nvm\"\n[ -s \"$NVM_DIR/nvm.sh\" ] && \\. \"$NVM_DIR/nvm.sh\"\n# >>> runa >>>\n").unwrap();
        let m = Manager::with_paths(crate::Paths::with_root(h.join("runa"))).unwrap();
        adopt(
            &m,
            "node",
            "20.1.0",
            &nvm.join("versions/node/v20.1.0"),
            "nvm",
        );
        m.set_global("node", "20.1.0").unwrap();
        let p = probe(&h, vec![]);
        let i = info_one(&m, &p, "nvm");
        assert!(i.installed && i.origin == "script");
        assert_eq!(i.runtimes.len(), 1);
        let pl = plan(&m, &p, "nvm", "uninstall").unwrap();
        assert_eq!(pl.kind, PlanKind::Run);
        assert_eq!(pl.remove_dirs, vec![nvm.clone()]);
        assert_eq!(pl.rc_edits.len(), 1);
        assert_eq!(pl.rc_edits[0].lines.len(), 2);
        assert!(pl.runtimes[0].global);
        assert!(nvm.is_dir(), "plan changes nothing");
        let rep = execute(&m, &p, &pl, &mut |_| {}).unwrap();
        assert!(!nvm.exists());
        assert_eq!(
            std::fs::read_to_string(&rc).unwrap(),
            "alias ll='ls -l'\n# >>> runa >>>\n"
        );
        assert!(h.join(".zshrc.runa-bak").exists());
        assert!(m.installed("node").is_empty());
        assert_eq!(m.global_version("node"), None);
        assert_eq!(rep.notices.len(), 1);
    }

    /// `keep_data`: nvm's own files and rc lines go, `versions/` stays, the runtime stays
    /// registered (provider `manual`) and global.
    #[test]
    fn uninstall_nvm_keep_data() {
        let t = tempfile::tempdir().unwrap();
        let h = platform::canonicalize(t.path());
        let nvm = h.join(".nvm");
        let home = nvm.join("versions/node/v20.1.0");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        std::fs::create_dir_all(nvm.join(".git")).unwrap();
        std::fs::write(nvm.join("nvm.sh"), "nvm() { echo 0.40.3; }\n").unwrap();
        std::fs::write(nvm.join("nvm-exec"), "").unwrap();
        let rc = h.join(".bashrc");
        std::fs::write(&rc, "export NVM_DIR=\"$HOME/.nvm\"\n").unwrap();
        let m = Manager::with_paths(crate::Paths::with_root(h.join("runa"))).unwrap();
        adopt(&m, "node", "20.1.0", &home, "nvm");
        m.set_global("node", "20.1.0").unwrap();
        let p = probe(&h, vec![]);
        let full = plan(&m, &p, "nvm", "uninstall").unwrap();
        assert_eq!(full.data_runtimes, vec![home.clone()]);
        assert!(!full.keep_data);
        let pl = plan_with(&m, &p, "nvm", "uninstall", true).unwrap();
        assert!(pl.keep_data);
        assert_eq!(
            pl.remove_dirs,
            vec![nvm.join(".git"), nvm.join("nvm-exec"), nvm.join("nvm.sh")]
        );
        assert_eq!(pl.rc_edits.len(), 1);
        let rep = execute(&m, &p, &pl, &mut |_| {}).unwrap();
        assert!(home.is_dir() && !nvm.join("nvm.sh").exists());
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), "");
        assert_eq!(m.installed("node"), vec!["20.1.0".to_string()]);
        assert_eq!(
            m.receipt("node", "20.1.0").unwrap().provider.as_deref(),
            Some("manual")
        );
        assert_eq!(m.global_version("node").as_deref(), Some("20.1.0"));
        assert!(rep.unregistered.is_empty());
        // without any runtimes there is nothing to keep: keep_data falls back to a full uninstall
        let t2 = tempfile::tempdir().unwrap();
        let h2 = platform::canonicalize(t2.path());
        std::fs::create_dir_all(h2.join(".nvm")).unwrap();
        std::fs::write(h2.join(".nvm/nvm.sh"), "nvm() { echo 0.40.3; }\n").unwrap();
        let m2 = Manager::with_paths(crate::Paths::with_root(h2.join("runa"))).unwrap();
        let pl2 = plan_with(&m2, &probe(&h2, vec![]), "nvm", "uninstall", true).unwrap();
        assert!(!pl2.keep_data && pl2.data_runtimes.is_empty());
        assert_eq!(pl2.remove_dirs, vec![h2.join(".nvm")]);
    }

    /// brew-installed manager: brew uninstall runs (fake brew), the data dir with the
    /// runtimes is removed; a failing command removes and unregisters nothing.
    #[cfg(unix)]
    #[test]
    fn uninstall_and_upgrade_brew_fnm() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let h = platform::canonicalize(t.path());
        let cellar = h.join("Cellar/fnm/1.37.1/bin");
        std::fs::create_dir_all(&cellar).unwrap();
        let fnm = cellar.join("fnm");
        std::fs::write(&fnm, "#!/bin/sh\necho 'fnm 1.37.1'\n").unwrap();
        std::fs::set_permissions(&fnm, std::fs::Permissions::from_mode(0o755)).unwrap();
        let bin = h.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(&fnm, bin.join("fnm")).unwrap();
        let log = h.join("brew.log");
        let brew = bin.join("brew");
        let set_brew = |code: i32| {
            std::fs::write(
                &brew,
                format!(
                    "#!/bin/sh\necho \"$@\" >> '{}'\nexit {code}\n",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&brew, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let data = h.join(".local/share/fnm");
        std::fs::create_dir_all(data.join("node-versions/v22.1.0/installation/bin")).unwrap();
        let m = Manager::with_paths(crate::Paths::with_root(h.join("runa"))).unwrap();
        adopt(
            &m,
            "node",
            "22.1.0",
            &data.join("node-versions/v22.1.0/installation"),
            "fnm",
        );
        set_brew(0);
        let p = probe(&h, vec![bin.clone()]);
        let i = info_one(&m, &p, "fnm");
        assert_eq!(i.origin, "brew");
        assert_eq!(i.version.as_deref(), Some("1.37.1"));
        let up = plan(&m, &p, "fnm", "upgrade").unwrap();
        assert_eq!(up.command_line.as_deref(), Some("brew upgrade fnm"));
        let pl = plan(&m, &p, "fnm", "uninstall").unwrap();
        assert_eq!(pl.command_line.as_deref(), Some("brew uninstall fnm"));
        assert_eq!(pl.remove_dirs, vec![data.clone()]);
        set_brew(1);
        assert!(execute(&m, &p, &pl, &mut |_| {}).is_err());
        assert!(data.is_dir() && m.installed("node") == vec!["22.1.0".to_string()]);
        set_brew(0);
        execute(&m, &p, &up, &mut |_| {}).unwrap();
        execute(&m, &p, &pl, &mut |_| {}).unwrap();
        let l = std::fs::read_to_string(&log).unwrap();
        assert!(l.contains("upgrade fnm") && l.contains("uninstall fnm"));
        assert!(!data.exists());
        assert!(m.installed("node").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn system_or_unknown_origin_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let h = platform::canonicalize(t.path());
        // pyenv without .git and outside brew: unknown origin
        let bin = h.join(".pyenv/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("pyenv"), "#!/bin/sh\necho pyenv 2.4.0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin.join("pyenv"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let m = Manager::with_paths(crate::Paths::with_root(h.join("runa"))).unwrap();
        let p = probe(&h, vec![]);
        let pl = plan(&m, &p, "pyenv", "uninstall").unwrap();
        assert_eq!(pl.kind, PlanKind::Refuse);
        assert!(execute(&m, &p, &pl, &mut |_| {}).is_err());
        assert!(bin.join("pyenv").exists());
        // with a git checkout it is the pyenv-installer layout
        std::fs::create_dir_all(h.join(".pyenv/.git")).unwrap();
        let up = plan(&m, &p, "pyenv", "upgrade").unwrap();
        assert!(up.command_line.unwrap().ends_with("pull --ff-only"));
        assert!(plan(&m, &p, "mise", "uninstall").is_err(), "not installed");
    }
}
