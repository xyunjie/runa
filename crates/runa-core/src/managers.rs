//! Install new versions through the version manager the user already has (nvm,
//! nvm-windows, fnm, pyenv, pyenv-win, SDKMAN!, asdf, mise), then register the result so
//! it is managed (and later uninstalled) through that manager. Falls back to runa's own
//! download when no manager is available or the user prefers it.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

use crate::detect::{self, Detected};
use crate::installer::Progress;
use crate::locations::Env;
use crate::manager::{find_exe_in, Manager};
use crate::version::VersionReq;

/// Accepted installer names (`--via`, settings).
pub const VIAS: &[&str] = &[
    "auto",
    "runa",
    "nvm",
    "nvm-windows",
    "fnm",
    "pyenv",
    "pyenv-win",
    "sdkman",
    "asdf",
    "mise",
];

/// Managers able to install `tool`, in order of preference, on this platform.
pub fn candidates(tool: &str) -> Vec<&'static str> {
    candidates_for(tool, cfg!(windows))
}

pub fn candidates_for(tool: &str, windows: bool) -> Vec<&'static str> {
    match (tool, windows) {
        ("node", false) => vec!["nvm", "fnm", "mise", "asdf"],
        ("node", true) => vec!["nvm-windows", "fnm", "mise"],
        ("python", false) => vec!["pyenv", "mise", "asdf"],
        ("python", true) => vec!["pyenv-win", "mise"],
        ("java", false) => vec!["sdkman", "mise"],
        ("java", true) => vec!["mise"],
        ("go", false) => vec!["mise", "asdf"],
        ("go", true) => vec!["mise"],
        _ => vec![],
    }
}

/// asdf plugin name for a runa tool.
pub fn asdf_plugin(tool: &str) -> &str {
    match tool {
        "node" => "nodejs",
        "go" => "golang",
        t => t,
    }
}

/// What a manager installation needs: where the manager is and its data dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// Executable (or, for nvm / SDKMAN!, the init script that defines the function).
    pub entry: PathBuf,
    /// Data dir (NVM_DIR, NVM_HOME, FNM_DIR, PYENV_ROOT, SDKMAN_DIR, ASDF_DATA_DIR, ...).
    pub store: PathBuf,
}

/// The machine as seen by manager detection (injectable for tests).
#[derive(Debug, Clone)]
pub struct Probe {
    pub env: Env,
    /// Search path (system PATH without runa's own dirs).
    pub path: Vec<PathBuf>,
    /// Well-known bin dirs searched after `path`, so a manager installed a moment ago (or
    /// a GUI started without the login PATH) is found without restarting.
    pub fallback: Vec<PathBuf>,
}

/// Bin dirs package managers and install scripts put executables in.
pub fn fallback_bins(env: &Env) -> Vec<PathBuf> {
    let h = &env.home;
    let mut v: Vec<PathBuf> = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/home/linuxbrew/.linuxbrew/bin",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    v.push(h.join(".linuxbrew").join("bin"));
    v.push(h.join(".local").join("bin"));
    v.push(h.join(".cargo").join("bin"));
    v.push(
        env.var("SCOOP")
            .map(Path::to_path_buf)
            .unwrap_or_else(|| h.join("scoop"))
            .join("shims"),
    );
    if let Some(l) = &env.local_appdata {
        v.push(l.join("Microsoft").join("WinGet").join("Links"));
        v.push(l.join("Microsoft").join("WindowsApps"));
    }
    v
}

impl Probe {
    pub fn current(m: &Manager) -> Self {
        let env = Env::current();
        Probe {
            fallback: fallback_bins(&env),
            env,
            path: m.system_path(),
        }
    }

    pub fn find(&self, exe: &str, extra: &[PathBuf]) -> Option<PathBuf> {
        let mut dirs = extra.to_vec();
        dirs.extend(self.path.iter().cloned());
        dirs.extend(self.fallback.iter().cloned());
        find_exe_in(&dirs, exe)
    }

    fn first_existing(&self, provider: &str) -> Option<PathBuf> {
        let dirs = self.env.manager_dir(provider);
        dirs.iter()
            .find(|d| d.is_dir())
            .cloned()
            .or_else(|| dirs.first().cloned())
    }

    /// Locate `via` for installing `tool`; `None` when it is not usable here.
    pub fn locate(&self, via: &str, tool: &str) -> Option<Located> {
        let h = &self.env.home;
        match via {
            "nvm" => self
                .env
                .manager_dir("nvm")
                .into_iter()
                .map(|d| (d.join("nvm.sh"), d))
                .find(|(s, _)| s.is_file())
                .map(|(entry, store)| Located { entry, store }),
            "sdkman" => self
                .env
                .manager_dir("sdkman")
                .into_iter()
                .map(|d| (d.join("bin").join("sdkman-init.sh"), d))
                .find(|(s, _)| s.is_file())
                .map(|(entry, store)| Located { entry, store }),
            "nvm-windows" => {
                let store = self.first_existing("nvm-windows")?;
                let entry = self.find("nvm", std::slice::from_ref(&store))?;
                Some(Located { entry, store })
            }
            "fnm" => {
                let store = self.first_existing("fnm")?;
                let entry = self.find("fnm", &[store.clone(), h.join(".cargo").join("bin")])?;
                Some(Located { entry, store })
            }
            "pyenv" | "pyenv-win" => {
                let store = self.first_existing(via)?;
                let entry = self.find("pyenv", &[store.join("bin")])?;
                Some(Located { entry, store })
            }
            "asdf" => {
                let store = self.first_existing("asdf")?;
                // only when the plugin is installed: asdf cannot install without it
                if !store.join("plugins").join(asdf_plugin(tool)).is_dir() {
                    return None;
                }
                let entry = self.find("asdf", &[store.join("bin")])?;
                Some(Located { entry, store })
            }
            "mise" => {
                let store = self.first_existing("mise")?;
                let entry = self.find(
                    "mise",
                    &[h.join(".local").join("bin"), h.join(".cargo").join("bin")],
                )?;
                Some(Located { entry, store })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstallerOption {
    pub via: String,
    pub available: bool,
    /// Where it was found, or why it is unavailable.
    pub detail: String,
}

/// Which installer a new version of `tool` would use, and the alternatives (GUI / CLI).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstallerChoice {
    pub tool: String,
    /// `runa` or a manager name.
    pub chosen: String,
    /// The saved preference (`auto`, `runa`, or a manager).
    pub preference: String,
    pub options: Vec<InstallerOption>,
}

/// Decide the installer for `tool`. `pref`: `auto`, `runa` or a manager name. An explicit
/// manager that is not available is an error (never a silent fallback).
pub fn choose(probe: &Probe, tool: &str, pref: &str) -> Result<(String, Option<Located>)> {
    choose_for(probe, tool, pref, cfg!(windows))
}

fn choose_for(
    probe: &Probe,
    tool: &str,
    pref: &str,
    windows: bool,
) -> Result<(String, Option<Located>)> {
    let cands = candidates_for(tool, windows);
    match pref {
        "" | "auto" => Ok(cands
            .iter()
            .find_map(|v| probe.locate(v, tool).map(|l| (v.to_string(), Some(l))))
            .unwrap_or_else(|| ("runa".into(), None))),
        "runa" => Ok(("runa".into(), None)),
        v if !VIAS.contains(&v) => bail!("未知安装方式: {v}（可选: {}）", VIAS.join(" / ")),
        v if !cands.contains(&v) => bail!("{v} 不能在此平台上安装 {tool}"),
        v => match probe.locate(v, tool) {
            Some(l) => Ok((v.to_string(), Some(l))),
            None => bail!("未检测到可用的 {v}（安装 {tool} 需要它），可改用 --via runa"),
        },
    }
}

pub fn installer_choice(probe: &Probe, tool: &str, pref: &str) -> InstallerChoice {
    let mut options = vec![InstallerOption {
        via: "runa".into(),
        available: true,
        detail: "runa 直接下载".into(),
    }];
    for v in candidates(tool) {
        let l = probe.locate(v, tool);
        options.push(InstallerOption {
            via: v.into(),
            available: l.is_some(),
            detail: l
                .map(|l| l.entry.display().to_string())
                .unwrap_or_else(|| "未检测到".into()),
        });
    }
    let chosen = choose(probe, tool, pref)
        .map(|(v, _)| v)
        .unwrap_or_else(|_| format!("{pref}（不可用）"));
    InstallerChoice {
        tool: tool.into(),
        chosen,
        preference: if pref.is_empty() {
            "auto".into()
        } else {
            pref.into()
        },
        options,
    }
}

/// A command to run, plus what to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerCmd {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub display: String,
}

fn check_id(v: &str) -> Result<&str> {
    if v.is_empty()
        || v.starts_with('-')
        || !v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
    {
        bail!("无效的版本号: {v:?}");
    }
    Ok(v)
}

/// SDKMAN! identifier: `21.0.2` -> `21.0.2-tem` (Temurin); a full id is kept.
pub fn sdkman_id(version: &str) -> String {
    if version.contains('-') {
        version.to_string()
    } else {
        format!("{version}-tem")
    }
}

/// The manager's install command for `tool version`.
pub fn install_command(via: &str, tool: &str, version: &str, loc: &Located) -> Result<ManagerCmd> {
    check_id(version)?;
    let store = loc.store.display().to_string();
    let exe = |args: Vec<String>, env: Vec<(&str, String)>, shown: &str| ManagerCmd {
        program: loc.entry.clone(),
        display: format!("{shown} {}", args.join(" ")),
        args,
        env: env.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
    };
    let s = |x: &str| x.to_string();
    Ok(match via {
        // nvm / SDKMAN! are shell functions. The data dir and version are passed as
        // arguments and exported after the login profile ran, so a profile that sets its
        // own NVM_DIR / SDKMAN_DIR can never redirect the install.
        "nvm" => ManagerCmd {
            program: "bash".into(),
            args: vec![
                s("-lc"),
                s(r#"export NVM_DIR="$1"; . "$NVM_DIR/nvm.sh" && nvm install "$2""#),
                s("runa-nvm"),
                store,
                s(version),
            ],
            env: vec![],
            display: format!("nvm install {version}"),
        },
        "sdkman" => {
            let id = sdkman_id(version);
            check_id(&id)?;
            ManagerCmd {
                program: "bash".into(),
                args: vec![
                    s("-lc"),
                    s(
                        r#"export SDKMAN_DIR="$1" sdkman_auto_answer=true; . "$SDKMAN_DIR/bin/sdkman-init.sh" && sdk install "$2" "$3""#,
                    ),
                    s("runa-sdkman"),
                    store,
                    s(tool),
                    id.clone(),
                ],
                env: vec![],
                display: format!("sdk install {tool} {id}"),
            }
        }
        "nvm-windows" => exe(
            vec![s("install"), s(version)],
            vec![("NVM_HOME", store)],
            "nvm",
        ),
        "fnm" => exe(
            vec![s("install"), s(version)],
            vec![("FNM_DIR", store)],
            "fnm",
        ),
        "pyenv" => exe(
            vec![s("install"), s("-s"), s(version)],
            vec![("PYENV_ROOT", store)],
            "pyenv",
        ),
        "pyenv-win" => exe(
            vec![s("install"), s(version)],
            vec![("PYENV_ROOT", store)],
            "pyenv",
        ),
        "asdf" => exe(
            vec![s("install"), s(asdf_plugin(tool)), s(version)],
            vec![("ASDF_DATA_DIR", store)],
            "asdf",
        ),
        "mise" => exe(
            vec![s("install"), format!("{tool}@{version}")],
            vec![("MISE_DATA_DIR", store)],
            "mise",
        ),
        v => bail!("{v} 不是版本管理器"),
    })
}

/// Where the manager puts `tool version` (candidates, first existing wins).
pub fn expected_homes(via: &str, tool: &str, version: &str, store: &Path) -> Vec<PathBuf> {
    let vv = format!("v{}", version.trim_start_matches('v'));
    match via {
        "nvm" => vec![store.join("versions").join("node").join(&vv)],
        "nvm-windows" => vec![store.join(&vv)],
        "fnm" => vec![store.join("node-versions").join(&vv).join("installation")],
        "pyenv" | "pyenv-win" => vec![store.join("versions").join(version)],
        "sdkman" => vec![store.join("candidates").join(tool).join(sdkman_id(version))],
        "asdf" => {
            let d = store.join("installs").join(asdf_plugin(tool)).join(version);
            if tool == "go" {
                vec![d.join("go"), d]
            } else {
                vec![d]
            }
        }
        "mise" => vec![store.join("installs").join(tool).join(version)],
        _ => vec![],
    }
}

/// Run `cmd`, feeding every output line (stdout and stderr, `\r` progress included) to
/// `on_line`. Returns the last lines on failure.
pub fn run_streaming(cmd: &ManagerCmd, on_line: &mut dyn FnMut(String)) -> Result<()> {
    let mut child = Command::new(&cmd.program)
        .args(&cmd.args)
        .envs(cmd.env.iter().cloned())
        .env("RUNA_DISABLE_SHIM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("无法执行 {}", cmd.program.display()))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    for r in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for chunk in BufReader::new(r).split(b'\n').map_while(Result::ok) {
                let text = String::from_utf8_lossy(&chunk).to_string();
                // keep the final state of `\r`-redrawn progress lines
                if let Some(last) = text.split('\r').rev().find(|s| !s.trim().is_empty()) {
                    let _ = tx.send(last.trim_end().to_string());
                }
            }
        }));
    }
    drop(tx);
    let mut tail: std::collections::VecDeque<String> = Default::default();
    for line in rx {
        if tail.len() == 20 {
            tail.pop_front();
        }
        tail.push_back(line.clone());
        on_line(line);
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait()?;
    if !status.success() {
        bail!(
            "{} 执行失败（{status}）:\n{}",
            cmd.display,
            tail.into_iter().collect::<Vec<_>>().join("\n")
        );
    }
    Ok(())
}

/// Install exactly `version` of `tool` through `via` at `loc`, then register (adopt) it
/// under `version` with the manager as provider.
pub fn install_with_manager(
    m: &Manager,
    via: &str,
    loc: &Located,
    tool: &str,
    version: &str,
    progress: &mut dyn FnMut(Progress),
) -> Result<String> {
    let plugin = m.plugin(tool)?;
    let tool = plugin.name().to_string();
    let name = version.split('-').next().unwrap_or(version).to_string();
    if m.installed(&tool).contains(&name) {
        bail!("{tool} {name} 已安装");
    }
    let cmd = install_command(via, &tool, version, loc)?;
    progress(Progress::Manager {
        tool: tool.clone(),
        version: name.clone(),
        via: via.into(),
        command: cmd.display.clone(),
    });
    let t2 = tool.clone();
    run_streaming(&cmd, &mut |line| {
        progress(Progress::Output {
            tool: t2.clone(),
            line,
        })
    })?;
    let home = expected_homes(via, &tool, version, &loc.store)
        .into_iter()
        .find(|h| h.is_dir())
        .ok_or_else(|| anyhow!("{} 已执行，但找不到安装目录", cmd.display))?;
    let home = crate::platform::canonicalize(&home);
    let (exe, bin) = plugin
        .detect_exes()
        .into_iter()
        .find_map(|e| {
            let bin = detect::real_bin_dir(&home, e)?;
            Some((find_exe_in(std::slice::from_ref(&bin), e)?, bin))
        })
        .ok_or_else(|| anyhow!("{} 中找不到 {tool} 可执行文件", home.display()))?;
    let (provider, formula) = detect::classify(&home);
    let d = Detected {
        tool: tool.clone(),
        version: name.clone(),
        exe,
        found_in: bin,
        home,
        via: via.into(),
        provider,
        formula,
        adopted: false,
        adopted_as: None,
        disabled: false,
        system_default: false,
        runa_global: false,
        view_only: false,
    };
    let v = detect::adopt(m, &d, Some(&name))?;
    detect::invalidate_cache(m);
    progress(Progress::Done {
        tool,
        version: v.clone(),
    });
    Ok(v)
}

/// Install a new version: through the preferred / detected manager, else runa's download.
/// `via`: explicit choice (CLI `--via`), else the saved preference, else `auto`.
/// Returns `(version, installer used)`.
pub fn install(
    m: &Manager,
    tool: &str,
    spec: &str,
    via: Option<&str>,
    progress: &mut dyn FnMut(Progress),
) -> Result<(String, String)> {
    let plugin = m.plugin(tool)?;
    let tool = plugin.name().to_string();
    let pref = via
        .unwrap_or_else(|| m.settings.install_via(&tool))
        .to_string();
    let (chosen, loc) = choose(&Probe::current(m), &tool, &pref)?;
    let Some(loc) = loc else {
        return Ok((m.install(&tool, spec, progress)?, "runa".into()));
    };
    // resolve the exact version with runa's index (a full SDKMAN! id is kept as is)
    let version = if chosen == "sdkman" && spec.contains('-') {
        spec.to_string()
    } else {
        let req = VersionReq::parse(spec);
        if req == VersionReq::System {
            bail!("system 表示使用系统自带版本，无需安装");
        }
        progress(Progress::Resolving {
            tool: tool.clone(),
            spec: spec.into(),
        });
        plugin
            .resolve_remote(&m.net()?, &req)
            .with_context(|| format!("解析 {tool} {spec} 失败"))?
    };
    let v = install_with_manager(m, &chosen, &loc, &tool, &version, progress)?;
    Ok((v, chosen))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn script(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn candidates_per_platform() {
        assert_eq!(candidates_for("node", false)[0], "nvm");
        assert_eq!(candidates_for("node", true)[0], "nvm-windows");
        assert_eq!(candidates_for("python", true)[0], "pyenv-win");
        assert_eq!(candidates_for("java", false)[0], "sdkman");
        assert!(candidates_for("deno", false).is_empty());
    }

    #[test]
    fn commands_and_homes() {
        let store = Path::new("/s").join(".nvm");
        let loc = Located {
            entry: store.join("nvm.sh"),
            store: store.clone(),
        };
        let c = install_command("nvm", "node", "20.11.1", &loc).unwrap();
        assert_eq!(c.program, PathBuf::from("bash"));
        assert_eq!(c.args[0], "-lc");
        assert!(c.args[1].contains(r#"nvm install "$2""#));
        assert_eq!(c.args[3..], [store.display().to_string(), "20.11.1".into()]);
        assert_eq!(
            expected_homes("nvm", "node", "20.11.1", &store),
            vec![store.join("versions").join("node").join("v20.11.1")]
        );
        let c = install_command("sdkman", "java", "21.0.2", &loc).unwrap();
        assert_eq!(c.display, "sdk install java 21.0.2-tem");
        assert!(c.args[1].contains("sdkman_auto_answer=true"));
        assert_eq!(c.args[4..], ["java".to_string(), "21.0.2-tem".into()]);
        let c = install_command("mise", "go", "1.22.0", &loc).unwrap();
        assert_eq!(c.args, vec!["install".to_string(), "go@1.22.0".into()]);
        let c = install_command("asdf", "node", "20.1.0", &loc).unwrap();
        assert_eq!(c.args[1], "nodejs");
        let c = install_command("nvm-windows", "node", "20.1.0", &loc).unwrap();
        assert_eq!(c.args, vec!["install".to_string(), "20.1.0".into()]);
        assert_eq!(
            expected_homes("nvm-windows", "node", "20.1.0", &store),
            vec![store.join("v20.1.0")]
        );
        assert_eq!(
            expected_homes("fnm", "node", "20.1.0", &store),
            vec![store
                .join("node-versions")
                .join("v20.1.0")
                .join("installation")]
        );
        assert!(install_command("pyenv", "python", "3.12;rm -rf /", &loc).is_err());
        assert!(install_command("pyenv", "python", "-s", &loc).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn choose_prefers_detected_managers_and_honours_overrides() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let p0 = probe(h, vec![]);
        assert_eq!(choose_for(&p0, "node", "auto", false).unwrap().0, "runa");
        assert!(
            choose_for(&p0, "node", "nvm", false).is_err(),
            "explicit but missing"
        );
        assert!(choose_for(&p0, "node", "bogus", false).is_err());
        assert!(
            choose_for(&p0, "node", "sdkman", false).is_err(),
            "wrong tool"
        );
        // fnm on PATH with its data dir
        let bin = h.join("bin");
        script(&bin.join("fnm"), "#!/bin/sh\n");
        std::fs::create_dir_all(h.join(".local").join("share").join("fnm")).unwrap();
        let p1 = probe(h, vec![bin.clone()]);
        assert_eq!(choose_for(&p1, "node", "auto", false).unwrap().0, "fnm");
        // nvm wins over fnm once nvm.sh exists
        script(&h.join(".nvm").join("nvm.sh"), "");
        let (v, l) = choose_for(&p1, "node", "auto", false).unwrap();
        assert_eq!(v, "nvm");
        assert_eq!(l.unwrap().store, h.join(".nvm"));
        assert_eq!(choose_for(&p1, "node", "runa", false).unwrap().0, "runa");
        assert_eq!(choose_for(&p1, "node", "fnm", false).unwrap().0, "fnm");
        // asdf needs its plugin
        script(&bin.join("asdf"), "#!/bin/sh\n");
        std::fs::create_dir_all(h.join(".asdf")).unwrap();
        assert!(p1.locate("asdf", "go").is_none());
        std::fs::create_dir_all(h.join(".asdf").join("plugins").join("golang")).unwrap();
        assert!(p1.locate("asdf", "go").is_some());
        let c = installer_choice(&p1, "node", "auto");
        assert_eq!(c.chosen, "nvm");
        assert!(c.options.iter().any(|o| o.via == "fnm" && o.available));
        assert!(c.options.iter().any(|o| o.via == "mise" && !o.available));
    }

    #[cfg(unix)]
    fn mgr(t: &Path) -> Manager {
        Manager::with_paths(crate::Paths::with_root(t.join("runa"))).unwrap()
    }

    /// A fake pyenv installs into PYENV_ROOT; runa registers it as a pyenv version, and
    /// purging it later goes back through (the fake) pyenv.
    #[cfg(unix)]
    #[test]
    fn fake_pyenv_install_register_and_uninstall() {
        let t = tempfile::tempdir().unwrap();
        let root = crate::platform::canonicalize(t.path()).join(".pyenv");
        script(
            &root.join("bin").join("pyenv"),
            r#"#!/bin/sh
case "$1" in
install)
  v="$3"; echo "Downloading Python-$v..."; printf 'step 1\rstep 2\n' 1>&2
  mkdir -p "$PYENV_ROOT/versions/$v/bin"
  printf '#!/bin/sh\necho Python %s\n' "$v" > "$PYENV_ROOT/versions/$v/bin/python3"
  chmod +x "$PYENV_ROOT/versions/$v/bin/python3";;
uninstall) rm -rf "$PYENV_ROOT/versions/$3";;
esac
"#,
        );
        let m = mgr(t.path());
        let loc = Probe {
            env: Env {
                home: t.path().to_path_buf(),
                vars: vec![("PYENV_ROOT".into(), root.clone())],
                ..Default::default()
            },
            path: vec![],
            fallback: vec![],
        }
        .locate("pyenv", "python")
        .expect("fake pyenv found");
        let mut lines = Vec::new();
        let mut started = None;
        let v = install_with_manager(&m, "pyenv", &loc, "python", "3.12.1", &mut |p| match p {
            Progress::Output { line, .. } => lines.push(line),
            Progress::Manager { command, .. } => started = Some(command),
            _ => {}
        })
        .unwrap();
        assert_eq!(v, "3.12.1");
        assert_eq!(started.as_deref(), Some("pyenv install -s 3.12.1"));
        assert!(lines
            .iter()
            .any(|l| l.contains("Downloading Python-3.12.1")));
        assert!(lines.iter().any(|l| l == "step 2"), "{lines:?}");
        let r = m.receipt("python", "3.12.1").unwrap();
        assert_eq!(r.provider.as_deref(), Some("pyenv"));
        assert_eq!(
            r.home.as_deref(),
            Some(root.join("versions").join("3.12.1").as_path())
        );
        assert!(
            install_with_manager(&m, "pyenv", &loc, "python", "3.12.1", &mut |_| {}).is_err(),
            "already installed"
        );
        let plan =
            crate::uninstall::plan(&m, "python", "3.12.1", crate::uninstall::Mode::Purge).unwrap();
        assert_eq!(
            plan.command_line.as_deref(),
            Some("pyenv uninstall -f 3.12.1")
        );
        crate::uninstall::execute(&m, &plan).unwrap();
        assert!(!root.join("versions").join("3.12.1").exists());
        assert!(m.installed("python").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn manager_failure_registers_nothing_and_reports_output() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("mise");
        std::fs::create_dir_all(&root).unwrap();
        let loc = Located {
            entry: t.path().join("mise-bin"),
            store: root.clone(),
        };
        script(
            &loc.entry,
            "#!/bin/sh\necho 'mise ERROR no such version' 1>&2\nexit 3\n",
        );
        let m = mgr(t.path());
        let e = install_with_manager(&m, "mise", &loc, "go", "9.9.9", &mut |_| {}).unwrap_err();
        let msg = format!("{e:#}");
        assert!(
            msg.contains("no such version") && msg.contains("mise install go@9.9.9"),
            "{msg}"
        );
        assert!(m.installed("go").is_empty());
        // succeeded but nothing at the expected place
        script(&loc.entry, "#!/bin/sh\nexit 0\n");
        let e = install_with_manager(&m, "mise", &loc, "go", "9.9.9", &mut |_| {}).unwrap_err();
        assert!(format!("{e:#}").contains("找不到安装目录"));
    }

    #[cfg(unix)]
    #[test]
    fn fake_nvm_function_via_bash() {
        let t = tempfile::tempdir().unwrap();
        let nvm_dir = crate::platform::canonicalize(t.path()).join(".nvm");
        script(
            &nvm_dir.join("nvm.sh"),
            r#"nvm() { v="v$2"; d="$NVM_DIR/versions/node/$v/bin"; mkdir -p "$d"; printf '#!/bin/sh\necho %s\n' "$v" > "$d/node"; chmod +x "$d/node"; echo "Now using node $v"; }
"#,
        );
        let m = mgr(t.path());
        let loc = Located {
            entry: nvm_dir.join("nvm.sh"),
            store: nvm_dir.clone(),
        };
        let v = install_with_manager(&m, "nvm", &loc, "node", "20.11.1", &mut |_| {}).unwrap();
        assert_eq!(v, "20.11.1");
        let r = m.receipt("node", "20.11.1").unwrap();
        assert_eq!(r.provider.as_deref(), Some("nvm"));
    }

    #[test]
    fn settings_preference() {
        let mut s = crate::settings::Settings::default();
        assert_eq!(s.install_via("node"), "auto");
        s.set_install_via("node", "NVM").unwrap();
        assert_eq!(s.install_via("node"), "nvm");
        assert!(s.set_install_via("node", "brew").is_err());
        s.set_install_via("node", "auto").unwrap();
        assert!(s.install_via.is_empty());
    }
}
