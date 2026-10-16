//! Install a version manager itself (nvm, nvm-windows, fnm, pyenv, pyenv-win, SDKMAN!,
//! asdf, mise) so runtimes can then be installed through it.
//!
//! Package managers are preferred (Homebrew, winget, scoop), the official install script
//! is the fallback. Nothing is ever run with sudo: none of the methods need root, and
//! what each one changes outside its own directory (shell rc files, user environment) is
//! stated in [`Method::rc_note`] so the user can confirm with full knowledge.

use std::path::PathBuf;

use anyhow::{bail, Result};
use serde::Serialize;

use crate::managers::{self, ManagerCmd, Probe};

/// Managers runa can install.
pub const MANAGERS: &[&str] = &[
    "nvm",
    "nvm-windows",
    "fnm",
    "pyenv",
    "pyenv-win",
    "sdkman",
    "asdf",
    "mise",
    "rustup",
    "rbenv",
    "rvm",
];

/// Pinned versions of install scripts / sources.
pub const NVM_VERSION: &str = "v0.40.3";
pub const ASDF_VERSION: &str = "v0.14.1";

/// Whether `manager` exists for the platform.
pub fn supported_on(manager: &str, windows: bool) -> bool {
    match manager {
        "nvm" | "sdkman" | "pyenv" | "asdf" | "rbenv" | "rvm" => !windows,
        "rustup" => true,
        "nvm-windows" | "pyenv-win" => windows,
        "fnm" | "mise" => true,
        _ => false,
    }
}

/// Runtimes a manager installs (runa tool names).
pub fn tools_of(manager: &str) -> &'static [&'static str] {
    match manager {
        "nvm" | "nvm-windows" | "fnm" => &["node"],
        "pyenv" | "pyenv-win" => &["python"],
        "sdkman" => &["java"],
        "rustup" => &["rust"],
        "rbenv" | "rvm" => &["ruby"],
        "asdf" | "mise" => &[
            "node", "python", "java", "go", "rust", "ruby", "bun", "deno",
        ],
        _ => &[],
    }
}

/// One way of installing a manager.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Method {
    pub manager: String,
    /// `brew`, `winget`, `scoop`, `script`, `powershell` or `git`.
    pub method: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The exact command, as shown to the user.
    pub display: String,
    /// What it changes outside its own install dir (empty: nothing).
    pub rc_note: String,
    /// Extra remarks (prerequisites, UAC prompts, ...).
    pub note: String,
}

/// Helper programs a method may need, looked up on the probe.
fn helper(probe: &Probe, name: &str) -> Option<PathBuf> {
    probe.find(name, &[])
}

/// Every applicable method for `manager` on this machine, best first.
pub fn methods(probe: &Probe, manager: &str) -> Vec<Method> {
    methods_for(probe, manager, cfg!(windows))
}

pub fn methods_for(probe: &Probe, manager: &str, windows: bool) -> Vec<Method> {
    if !supported_on(manager, windows) {
        return vec![];
    }
    let s = |x: &str| x.to_string();
    let mut v = Vec::new();
    let pkg = |v: &mut Vec<Method>, method: &str, args: &[&str], rc: &str, note: &str| {
        if let Some(p) = helper(probe, method) {
            v.push(Method {
                manager: s(manager),
                method: s(method),
                program: p,
                args: args.iter().map(|a| s(a)).collect(),
                display: format!("{method} {}", args.join(" ")),
                rc_note: s(rc),
                note: s(note),
            });
        }
    };
    let winget = |id: &str| -> Vec<String> {
        vec![
            s("install"),
            s("--id"),
            s(id),
            s("--exact"),
            s("--accept-source-agreements"),
            s("--accept-package-agreements"),
        ]
    };
    let script = |v: &mut Vec<Method>, cmd: &str, rc: &str, note: &str| {
        let (Some(bash), Some(_)) = (helper(probe, "bash"), helper(probe, "curl")) else {
            return;
        };
        v.push(Method {
            manager: s(manager),
            method: s("script"),
            program: bash,
            args: vec![s("-c"), s(cmd)],
            display: s(cmd),
            rc_note: s(rc),
            note: s(note),
        });
    };
    match manager {
        "nvm" => script(
            &mut v,
            &format!(
                "curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/{NVM_VERSION}/install.sh | bash"
            ),
            "官方脚本会在 ~/.bashrc / ~/.zshrc / ~/.profile 末尾追加加载 nvm 的代码",
            "安装到 ~/.nvm（或 $NVM_DIR）",
        ),
        "nvm-windows" => {
            let a = winget("CoreyButler.NVMforWindows");
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            pkg(
                &mut v,
                "winget",
                &a,
                "安装程序会设置用户环境变量 NVM_HOME / NVM_SYMLINK 并修改 PATH",
                "可能弹出 UAC 授权窗口",
            );
            pkg(&mut v, "scoop", &["install", "nvm"], "", "");
        }
        "fnm" => {
            pkg(&mut v, "brew", &["install", "fnm"], "", "");
            let a = winget("Schniz.fnm");
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            pkg(&mut v, "winget", &a, "", "");
            pkg(&mut v, "scoop", &["install", "fnm"], "", "");
            if !windows {
                script(
                    &mut v,
                    "curl -fsSL https://fnm.vercel.app/install | bash -s -- --skip-shell",
                    "",
                    "--skip-shell：不修改 shell 配置文件",
                );
            }
        }
        "pyenv" => {
            pkg(&mut v, "brew", &["install", "pyenv"], "", "");
            script(
                &mut v,
                "curl -fsSL https://pyenv.run | bash",
                "",
                "安装到 ~/.pyenv，不修改 shell 配置文件（脚本只打印建议）；编译 Python 需要系统构建依赖",
            );
        }
        "pyenv-win" => {
            pkg(&mut v, "scoop", &["install", "pyenv"], "", "");
            if let Some(ps) = helper(probe, "powershell") {
                let cmd = "Invoke-WebRequest -UseBasicParsing -Uri 'https://raw.githubusercontent.com/pyenv-win/pyenv-win/master/pyenv-win/install-pyenv-win.ps1' -OutFile \"$env:TEMP\\install-pyenv-win.ps1\"; & \"$env:TEMP\\install-pyenv-win.ps1\"";
                v.push(Method {
                    manager: s(manager),
                    method: s("powershell"),
                    program: ps,
                    args: vec![
                        s("-NoProfile"),
                        s("-ExecutionPolicy"),
                        s("Bypass"),
                        s("-Command"),
                        s(cmd),
                    ],
                    display: format!("powershell -NoProfile -ExecutionPolicy Bypass -Command {cmd}"),
                    rc_note: s("会设置用户环境变量 PYENV / PYENV_ROOT / PYENV_HOME 并把 pyenv-win 加入用户 PATH"),
                    note: s("安装到 %USERPROFILE%\\.pyenv"),
                });
            }
        }
        "sdkman" => script(
            &mut v,
            "curl -s \"https://get.sdkman.io?rcupdate=true\" | bash",
            "rcupdate=true：会在 ~/.bashrc 和 ~/.zshrc 中加入 SDKMAN! 初始化代码",
            "需要 zip / unzip；安装到 ~/.sdkman（或 $SDKMAN_DIR）",
        ),
        "asdf" => {
            pkg(&mut v, "brew", &["install", "asdf"], "", "");
            if let Some(git) = helper(probe, "git") {
                let dest = probe
                    .env
                    .manager_dir("asdf")
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| probe.env.home.join(".asdf"));
                let args = vec![
                    s("clone"),
                    s("https://github.com/asdf-vm/asdf.git"),
                    dest.display().to_string(),
                    s("--branch"),
                    s(ASDF_VERSION),
                ];
                v.push(Method {
                    manager: s(manager),
                    method: s("git"),
                    program: git,
                    display: format!("git {}", args.join(" ")),
                    args,
                    rc_note: String::new(),
                    note: format!("安装 asdf {ASDF_VERSION} 到 {}，不修改 shell 配置文件", dest.display()),
                });
            }
        }
        "rustup" => {
            if windows {
                let a = winget("Rustlang.Rustup");
                let a: Vec<&str> = a.iter().map(String::as_str).collect();
                pkg(
                    &mut v,
                    "winget",
                    &a,
                    "rustup-init 会把 %USERPROFILE%\\.cargo\\bin 加入用户 PATH",
                    "需要 Visual Studio C++ 生成工具才能链接；安装到 %USERPROFILE%\\.rustup",
                );
                pkg(&mut v, "scoop", &["install", "rustup"], "", "");
            } else {
                script(
                    &mut v,
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path",
                    "",
                    "官方 rustup-init；--no-modify-path：不修改 shell 配置文件。安装到 ~/.rustup 和 ~/.cargo，并装好 stable 工具链",
                );
            }
        }
        "rbenv" => {
            pkg(&mut v, "brew", &["install", "rbenv", "ruby-build"], "", "");
            if let (Some(bash), Some(_)) = (helper(probe, "bash"), helper(probe, "git")) {
                let dest = probe
                    .env
                    .manager_dir("rbenv")
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| probe.env.home.join(".rbenv"));
                let d = dest.display().to_string();
                let cmd = format!(
                    "git clone https://github.com/rbenv/rbenv.git {d} && git clone https://github.com/rbenv/ruby-build.git {d}/plugins/ruby-build"
                );
                v.push(Method {
                    manager: s(manager),
                    method: s("git"),
                    program: bash,
                    args: vec![s("-c"), cmd.clone()],
                    display: cmd,
                    rc_note: String::new(),
                    note: format!("安装 rbenv 和 ruby-build 到 {d}，不修改 shell 配置文件；编译 Ruby 需要系统构建依赖"),
                });
            }
        }
        // rvm's installer needs GPG key import and edits several rc files: not offered
        "rvm" => {}
        "mise" => {
            pkg(&mut v, "brew", &["install", "mise"], "", "");
            let a = winget("jdx.mise");
            let a: Vec<&str> = a.iter().map(String::as_str).collect();
            pkg(&mut v, "winget", &a, "", "");
            pkg(&mut v, "scoop", &["install", "mise"], "", "");
            if !windows {
                script(
                    &mut v,
                    "curl -fsSL https://mise.run | sh",
                    "",
                    "安装到 ~/.local/bin/mise，不修改 shell 配置文件",
                );
            }
        }
        _ => {}
    }
    v
}

/// Whether `manager` is installed (for asdf: regardless of plugins).
pub fn is_installed(probe: &Probe, manager: &str) -> bool {
    match manager {
        "asdf" => {
            let bins: Vec<PathBuf> = probe
                .env
                .manager_dir("asdf")
                .iter()
                .map(|d| d.join("bin"))
                .collect();
            probe.find("asdf", &bins).is_some()
        }
        "rbenv" => {
            let bins: Vec<PathBuf> = probe
                .env
                .manager_dir("rbenv")
                .iter()
                .map(|d| d.join("bin"))
                .collect();
            probe.find("rbenv", &bins).is_some()
        }
        m => tools_of(m)
            .first()
            .is_some_and(|t| probe.locate(m, t).is_some()),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManagerStatus {
    pub name: String,
    pub tools: Vec<String>,
    pub supported: bool,
    pub installed: bool,
    /// The method `install` would use (`None`: nothing applicable on this machine).
    pub method: Option<Method>,
}

pub fn status(probe: &Probe) -> Vec<ManagerStatus> {
    status_for(probe, cfg!(windows))
}

pub fn status_for(probe: &Probe, windows: bool) -> Vec<ManagerStatus> {
    MANAGERS
        .iter()
        .map(|m| ManagerStatus {
            name: m.to_string(),
            tools: tools_of(m).iter().map(|t| t.to_string()).collect(),
            supported: supported_on(m, windows),
            installed: is_installed(probe, m),
            method: methods_for(probe, m, windows).into_iter().next(),
        })
        .collect()
}

/// The method `install` will use, or why there is none.
pub fn plan(probe: &Probe, manager: &str) -> Result<Method> {
    if !MANAGERS.contains(&manager) {
        bail!(
            "未知的版本管理器: {manager}（可选: {}）",
            MANAGERS.join(" / ")
        );
    }
    if !supported_on(manager, cfg!(windows)) {
        bail!("{manager} 不支持当前平台");
    }
    if manager == "rvm" {
        bail!("runa 不安装 rvm（官方安装需要导入 GPG 密钥并修改多个 shell 配置文件）；推荐改用 rbenv，已安装的 rvm 会被自动识别");
    }
    methods(probe, manager).into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "没有可用的安装方式：需要 {}",
            if cfg!(windows) {
                "winget、scoop 或 PowerShell"
            } else {
                "Homebrew，或 curl + bash（asdf 还可用 git）"
            }
        )
    })
}

/// Run `method`, streaming its output, then confirm the manager is detected right away
/// (nvm / SDKMAN! are found by their init scripts, so no new shell is needed). For asdf,
/// `tool`'s plugin is added so it can install that runtime immediately.
pub fn install(
    probe: &Probe,
    method: &Method,
    tool: Option<&str>,
    on_line: &mut dyn FnMut(String),
) -> Result<()> {
    let cmd = ManagerCmd {
        program: method.program.clone(),
        args: method.args.clone(),
        env: vec![("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into())],
        display: method.display.clone(),
    };
    managers::run_streaming(&cmd, on_line)?;
    if !is_installed(probe, &method.manager) {
        bail!(
            "{} 已执行完成，但仍未检测到 {}；可能需要重新打开终端或检查输出",
            method.display,
            method.manager
        );
    }
    if method.manager == "asdf" {
        if let Some(t) = tool {
            let bins: Vec<PathBuf> = probe
                .env
                .manager_dir("asdf")
                .iter()
                .map(|d| d.join("bin"))
                .collect();
            let asdf = probe.find("asdf", &bins).expect("checked above");
            let plugin = managers::asdf_plugin(t).to_string();
            let mut env = vec![];
            if let Some(d) = probe.env.manager_dir("asdf").into_iter().next() {
                env.push(("ASDF_DATA_DIR".to_string(), d.display().to_string()));
            }
            managers::run_streaming(
                &ManagerCmd {
                    program: asdf,
                    args: vec!["plugin".into(), "add".into(), plugin.clone()],
                    env,
                    display: format!("asdf plugin add {plugin}"),
                },
                on_line,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locations::Env;
    use std::path::Path;

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
    fn platform_support() {
        assert!(supported_on("sdkman", false) && !supported_on("sdkman", true));
        assert!(supported_on("nvm-windows", true) && !supported_on("nvm-windows", false));
        assert!(supported_on("mise", true) && supported_on("mise", false));
    }

    #[cfg(unix)]
    #[test]
    fn methods_prefer_package_managers_then_scripts() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let bin = h.join("bin");
        let p = probe(h, vec![bin.clone()]);
        assert!(
            methods_for(&p, "fnm", false).is_empty(),
            "no helpers at all"
        );
        assert!(plan(&p, "fnm").is_err());
        script(&bin.join("curl"), "#!/bin/sh\n");
        script(&bin.join("bash"), "#!/bin/sh\n");
        let m = methods_for(&p, "fnm", false);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].method, "script");
        assert!(m[0].display.contains("--skip-shell"));
        script(&bin.join("brew"), "#!/bin/sh\n");
        let m = methods_for(&p, "fnm", false);
        assert_eq!(m[0].display, "brew install fnm");
        assert_eq!(m[1].method, "script");
        let nvm = &methods_for(&p, "nvm", false)[0];
        assert!(nvm
            .display
            .contains(&format!("nvm/{NVM_VERSION}/install.sh")));
        assert!(nvm.rc_note.contains(".bashrc"), "rc edits are disclosed");
        let sdk = &methods_for(&p, "sdkman", false)[0];
        assert!(sdk.display.contains("rcupdate=true") && !sdk.rc_note.is_empty());
        assert!(
            methods_for(&p, "sdkman", true).is_empty(),
            "no SDKMAN! on Windows"
        );
        // Windows: winget / scoop
        script(&bin.join("winget"), "#!/bin/sh\n");
        script(&bin.join("scoop"), "#!/bin/sh\n");
        let w = methods_for(&p, "nvm-windows", true);
        assert_eq!(w[0].method, "winget");
        assert!(w[0].args.contains(&"CoreyButler.NVMforWindows".to_string()));
        assert_eq!(w[1].display, "scoop install nvm");
        let py = methods_for(&p, "pyenv-win", true);
        assert_eq!(py[0].display, "scoop install pyenv");
        assert!(!methods_for(&p, "mise", true)
            .iter()
            .any(|m| m.method == "script"));
        // nothing ever runs through sudo
        for name in MANAGERS {
            for w in [false, true] {
                assert!(methods_for(&p, name, w)
                    .iter()
                    .all(|m| !m.display.contains("sudo")));
            }
        }
    }

    /// A fake `brew install fnm` drops an fnm binary: detected right after.
    #[cfg(unix)]
    #[test]
    fn install_with_fake_brew_is_detected_immediately() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let bin = h.join("bin");
        let fnm_dir = h.join(".local").join("share").join("fnm");
        script(
            &bin.join("brew"),
            &format!(
                "#!/bin/sh\necho \"==> Installing $2\"\nmkdir -p '{d}'\nprintf '#!/bin/sh\\n' > '{b}/fnm'\nchmod +x '{b}/fnm'\n",
                d = fnm_dir.display(),
                b = bin.display()
            ),
        );
        let p = probe(h, vec![bin.clone()]);
        assert!(!is_installed(&p, "fnm"));
        let m = plan(&p, "fnm").unwrap();
        let mut out = vec![];
        install(&p, &m, Some("node"), &mut |l| out.push(l)).unwrap();
        assert!(out.iter().any(|l| l.contains("Installing fnm")));
        assert!(is_installed(&p, "fnm"));
        assert_eq!(managers::choose(&p, "node", "auto").unwrap().0, "fnm");
    }

    /// nvm is a shell function: once the installer wrote nvm.sh it is usable at once.
    #[cfg(unix)]
    #[test]
    fn shell_function_manager_detected_without_restart() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let fake = h.join("fake-install.sh");
        script(
            &fake,
            &format!(
                "#!/bin/sh\nmkdir -p '{0}/.nvm'\necho 'nvm() {{ :; }}' > '{0}/.nvm/nvm.sh'\n",
                h.display()
            ),
        );
        let p = probe(h, vec![]);
        let m = Method {
            manager: "nvm".into(),
            method: "script".into(),
            program: fake,
            args: vec![],
            display: "fake".into(),
            rc_note: String::new(),
            note: String::new(),
        };
        install(&p, &m, None, &mut |_| {}).unwrap();
        assert!(is_installed(&p, "nvm"));
        // a method that "succeeds" without installing anything is reported
        let mut bad = m.clone();
        bad.manager = "mise".into();
        bad.program = PathBuf::from("/bin/true");
        assert!(install(&p, &bad, None, &mut |_| {}).is_err());
    }

    /// asdf: installing it also adds the plugin for the runtime being installed.
    #[cfg(unix)]
    #[test]
    fn asdf_install_adds_plugin() {
        let t = tempfile::tempdir().unwrap();
        let h = t.path();
        let bin = h.join("bin");
        let asdf =
            "#!/bin/sh\\n[ \"$1\" = plugin ] && mkdir -p \"$ASDF_DATA_DIR/plugins/$3\"\\nexit 0\\n";
        script(
            &bin.join("brew"),
            &format!(
                "#!/bin/sh\nmkdir -p '{h}/.asdf'\nprintf '{asdf}' > '{b}/asdf'\nchmod +x '{b}/asdf'\n",
                h = h.display(),
                b = bin.display()
            ),
        );
        let p = probe(h, vec![bin]);
        let m = plan(&p, "asdf").unwrap();
        install(&p, &m, Some("go"), &mut |_| {}).unwrap();
        assert!(h.join(".asdf").join("plugins").join("golang").is_dir());
        assert!(p.locate("asdf", "go").is_some());
    }
}
