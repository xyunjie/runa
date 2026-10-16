//! Uninstall planning and execution.
//!
//! Downloaded versions live in `installs/` and are simply deleted. Adopted versions are
//! owned by someone else, so the user picks:
//! - [`Mode::Forget`]: remove runa's record only, keep the files;
//! - [`Mode::Purge`]: complete uninstall through the owning tool (`brew uninstall`,
//!   `nvm uninstall`, ...), or by deleting the directory for manual installs.
//!
//! [`plan`] never changes anything, so the GUI can show exactly what will happen and the
//! CLI can offer `--dry-run`. [`execute`] carries the plan out.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::detect::{self, classify};
use crate::installer;
use crate::locations::{self, Env, Owner};
use crate::manager::{find_exe_in, Manager};
use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Downloads: delete. Adopted: refuse and ask for an explicit choice.
    Auto,
    /// Remove management only (receipt / legacy links), keep the files.
    Forget,
    /// Complete uninstall via the provider.
    Purge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    /// Delete a runa-owned install dir.
    DeleteInstall,
    /// Drop runa's record of an adopted runtime.
    Forget,
    /// Run the owning tool's uninstall command.
    ProviderCommand,
    /// Delete a manually installed directory.
    RemoveDir,
    /// Not possible (system runtime).
    Refuse,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallPlan {
    pub tool: String,
    pub version: String,
    pub kind: PlanKind,
    pub provider: Option<String>,
    /// argv executed for [`PlanKind::ProviderCommand`].
    pub command: Option<Vec<String>>,
    /// Extra environment for `command`.
    pub env: Vec<(String, String)>,
    /// What to show / run by hand (a shell command line).
    pub command_line: Option<String>,
    /// Everything that will be deleted (the runtime and runa's own records).
    pub paths: Vec<PathBuf>,
    /// The runtime home that disappears (purge) or is kept (forget).
    pub home: Option<PathBuf>,
    /// Needs administrator rights: runa will not sudo, the user runs `command_line`.
    pub needs_root: bool,
    pub summary: String,
}

/// Quote for display in the platform's shell (POSIX sh, or PowerShell on Windows).
fn q(s: &str) -> String {
    if cfg!(windows) {
        q_powershell(s)
    } else {
        q_sh(s)
    }
}

fn q_powershell(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+@/\\:,".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "''"))
}

fn q_sh(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+@/=:,".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn safe_id(id: &str) -> Result<&str> {
    if id.is_empty()
        || id.starts_with('-')
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-@".contains(c))
    {
        bail!("无法识别的版本标识: {id:?}");
    }
    Ok(id)
}

/// Locate a helper binary: PATH (without runa shims), then common install locations.
fn find_tool(m: &Manager, name: &str, extra: &[PathBuf]) -> String {
    let mut dirs: Vec<PathBuf> = extra.to_vec();
    dirs.extend(m.system_path());
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    dirs.push("/home/linuxbrew/.linuxbrew/bin".into());
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".linuxbrew").join("bin"));
    }
    find_exe_in(&dirs, name)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Directory to delete for a manual install: the whole macOS bundle for
/// `<bundle>/Contents/Home`, otherwise the home itself.
pub fn removal_root(home: &Path) -> PathBuf {
    let home = if home.ends_with("Contents/Home/jre") {
        home.parent().unwrap_or(home)
    } else {
        home
    };
    if home.ends_with("Contents/Home") {
        if let Some(b) = home.parent().and_then(Path::parent) {
            return b.to_path_buf();
        }
    }
    home.to_path_buf()
}

/// Whether the current user can delete entries in `dir`, decided from metadata only so
/// planning (and `--dry-run`) never writes anything. Unix: owner == the user's home dir
/// owner with u+w, or world-writable without the sticky bit.
fn writable(dir: &Path) -> bool {
    let Ok(md) = std::fs::metadata(dir) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let me = dirs::home_dir()
            .and_then(|h| std::fs::metadata(h).ok())
            .map(|m| m.uid());
        let mode = md.mode();
        (Some(md.uid()) == me && mode & 0o200 != 0) || (mode & 0o002 != 0 && mode & 0o1000 == 0)
    }
    #[cfg(not(unix))]
    {
        !md.permissions().readonly()
    }
}

/// Never delete these: too shallow, a shared prefix, the user's home or one of its
/// parents, an OS-owned path, or something a Windows installer owns.
fn dangerous(p: &Path) -> bool {
    p.components().count() < 3
        || detect::is_system_prefix(p)
        || dirs::home_dir()
            .map(|h| platform::path_within(&h, p))
            .unwrap_or(false)
        || matches!(classify(p).0.as_str(), "system" | "installer")
}

/// Real home of an adopted version. Registry receipts carry it; for legacy (symlink)
/// adoptions the runtime is re-resolved, because the recorded origin may be a shared bin
/// dir (`/opt/homebrew/bin`) or a stub prefix (`/usr`) that must never be deleted.
fn adopted_home(m: &Manager, tool: &str, version: &str, r: &installer::Receipt) -> Option<PathBuf> {
    let legacy = std::fs::symlink_metadata(m.paths.install_dir(tool, version)).is_ok();
    let h = match (&r.home, legacy) {
        (Some(h), false) => h.clone(),
        _ => {
            let plugin = m.plugin(tool).ok()?;
            detect::resolve_legacy(m, &plugin, version, r)?.0
        }
    };
    Some(platform::canonicalize(&h))
}

/// Work out what uninstalling `tool version` would do. Never changes anything.
pub fn plan(m: &Manager, tool: &str, version: &str, mode: Mode) -> Result<UninstallPlan> {
    installer::validate_version_name(version)?;
    let tool = m.plugin(tool)?.name().to_string();
    let dir = m.paths.install_dir(&tool, version);
    let receipt_file = installer::receipt_path(&m.paths, &tool, version);
    if !m.is_installed(&tool, version) && std::fs::symlink_metadata(&dir).is_err() {
        bail!("{tool} {version} 未安装");
    }
    let r = m.receipt(&tool, version).unwrap_or_default();
    let mut p = UninstallPlan {
        tool: tool.clone(),
        version: version.to_string(),
        kind: PlanKind::DeleteInstall,
        provider: None,
        command: None,
        env: vec![],
        command_line: None,
        paths: vec![],
        home: None,
        needs_root: false,
        summary: String::new(),
    };
    let mut runa_paths = if r.is_adopted() {
        detect::artifacts(&dir)
    } else {
        vec![dir.clone()]
    };
    if receipt_file.exists() {
        runa_paths.push(receipt_file);
    }
    if !r.is_adopted() {
        if mode == Mode::Forget {
            bail!("{tool} {version} 是由 runa 下载的版本，没有可保留的原始安装，请直接卸载");
        }
        p.paths = runa_paths;
        p.home = Some(dir.clone());
        p.summary = format!("删除 {}", dir.display());
        return Ok(p);
    }
    let home = adopted_home(m, &tool, version, &r);
    p.home = home.clone();
    let env = Env::current();
    let owner = home
        .as_deref()
        .map(|h| locations::owner(h, &env))
        .unwrap_or_else(|| Owner {
            provider: "manual".into(),
            ..Default::default()
        });
    // the location decides; an old receipt only fills in when the location says nothing
    let provider = match (&r.provider, owner.provider.as_str()) {
        (Some(rp), "manual") => rp.clone(),
        _ => owner.provider.clone(),
    };
    p.provider = Some(provider.clone());
    match mode {
        Mode::Auto => bail!(
            "{tool} {version} 是接管的版本（{provider}），请选择 --forget（只移除管理）或 --purge（彻底卸载）"
        ),
        Mode::Forget => {
            p.kind = PlanKind::Forget;
            p.paths = runa_paths;
            p.summary = match &home {
                Some(h) => format!("只移除 runa 的管理记录，保留 {}", h.display()),
                None => "只移除 runa 的管理记录".into(),
            };
            return Ok(p);
        }
        Mode::Purge => {}
    }
    let home = home.ok_or_else(|| {
        anyhow!("无法定位 {tool} {version} 的真实安装位置，只能移除管理（--forget）")
    })?;
    let user_home = env.home.clone();
    let id =
        || -> Result<String> { safe_id(owner.id.as_deref().unwrap_or("")).map(str::to_string) };
    let package = || -> Result<String> {
        safe_id(owner.package.as_deref().unwrap_or("")).map(str::to_string)
    };
    let store = || -> Result<PathBuf> {
        owner
            .store
            .clone()
            .ok_or_else(|| anyhow!("无法确定 {provider} 的数据目录"))
    };
    let disp = |p: &Path| p.display().to_string();
    p.kind = PlanKind::ProviderCommand;
    match provider.as_str() {
        "homebrew" => {
            let formula = r
                .formula
                .clone()
                .or(owner.package.clone())
                .ok_or_else(|| anyhow!("无法确定 Homebrew formula"))?;
            safe_id(&formula)?;
            // <prefix>/Cellar -> <prefix>/bin/brew (works for Linuxbrew too)
            let extra: Vec<PathBuf> = owner
                .store
                .as_deref()
                .and_then(Path::parent)
                .map(|p| p.join("bin"))
                .into_iter()
                .collect();
            p.command = Some(vec![
                find_tool(m, "brew", &extra),
                "uninstall".into(),
                formula.clone(),
            ]);
            p.command_line = Some(format!("brew uninstall {}", q(&formula)));
        }
        "nvm" => {
            let id = id()?;
            let script = format!(
                "export NVM_DIR={}; source \"$NVM_DIR/nvm.sh\" && nvm uninstall {}",
                q(&disp(&store()?)),
                q(&id)
            );
            p.command = Some(vec!["bash".into(), "-c".into(), script.clone()]);
            p.command_line = Some(format!("bash -c {}", q(&script)));
        }
        "nvm-windows" => {
            // nvm-windows names versions without the leading "v"
            let id = id()?;
            let ver = id.trim_start_matches('v').to_string();
            let root = store()?;
            p.env.push(("NVM_HOME".into(), disp(&root)));
            p.command = Some(vec![
                find_tool(m, "nvm", std::slice::from_ref(&root)),
                "uninstall".into(),
                ver.clone(),
            ]);
            p.command_line = Some(format!("nvm uninstall {}", q(&ver)));
        }
        "pyenv" | "pyenv-win" => {
            let id = id()?;
            let root = store()?;
            let exe = find_tool(m, "pyenv", &[root.join("bin")]);
            p.env.push(("PYENV_ROOT".into(), disp(&root)));
            if provider == "pyenv" {
                p.command = Some(vec![exe, "uninstall".into(), "-f".into(), id.clone()]);
                p.command_line = Some(format!("pyenv uninstall -f {}", q(&id)));
            } else {
                p.command = Some(vec![exe, "uninstall".into(), id.clone()]);
                p.command_line = Some(format!("pyenv uninstall {}", q(&id)));
            }
        }
        "sdkman" => {
            let (cand, id) = (package()?, id()?);
            let script = format!(
                "export SDKMAN_DIR={}; source \"$SDKMAN_DIR/bin/sdkman-init.sh\" && sdk uninstall {} {}",
                q(&disp(&store()?)),
                q(&cand),
                q(&id)
            );
            p.command = Some(vec!["bash".into(), "-c".into(), script.clone()]);
            p.command_line = Some(format!("bash -c {}", q(&script)));
        }
        "asdf" => {
            let (plugin, id) = (package()?, id()?);
            let data = store()?;
            let exe = find_tool(m, "asdf", &[data.join("bin")]);
            p.env.push(("ASDF_DATA_DIR".into(), disp(&data)));
            p.command = Some(vec![exe, "uninstall".into(), plugin.clone(), id.clone()]);
            p.command_line = Some(format!("asdf uninstall {} {}", q(&plugin), q(&id)));
        }
        "mise" => {
            let spec = format!("{}@{}", package()?, id()?);
            let data = store()?;
            let exe = find_tool(
                m,
                "mise",
                &[
                    user_home.join(".local").join("bin"),
                    user_home.join(".cargo").join("bin"),
                ],
            );
            p.env.push(("MISE_DATA_DIR".into(), disp(&data)));
            p.command = Some(vec![exe, "uninstall".into(), spec.clone()]);
            p.command_line = Some(format!("mise uninstall {}", q(&spec)));
        }
        "fnm" => {
            let id = id()?;
            let fnm_dir = store()?;
            let exe = find_tool(
                m,
                "fnm",
                &[
                    fnm_dir.clone(),
                    user_home.join(".local").join("share").join("fnm"),
                    user_home.join(".cargo").join("bin"),
                ],
            );
            p.env.push(("FNM_DIR".into(), disp(&fnm_dir)));
            p.command = Some(vec![exe, "uninstall".into(), id.clone()]);
            p.command_line = Some(format!("fnm uninstall {}", q(&id)));
        }
        "scoop" | "scoop-global" => {
            // scoop removes the whole app (every version it keeps), not one version dir
            let app = package()?;
            let root = store()?;
            let global = provider == "scoop-global";
            let exe = find_tool(
                m,
                "scoop",
                &[root.join("shims"), user_home.join("scoop").join("shims")],
            );
            let mut argv = vec![exe, "uninstall".into()];
            if global {
                argv.push("-g".into());
            }
            argv.push(app.clone());
            p.command_line = Some(format!(
                "scoop uninstall {}{}",
                if global { "-g " } else { "" },
                q(&app)
            ));
            p.command = Some(argv);
            p.needs_root = global;
            p.paths = vec![root.join("apps").join(&app)];
            p.paths.extend(runa_paths);
            p.summary = format!(
                "通过 scoop 彻底卸载 {app}（scoop 会删除该应用的所有版本）：{}",
                p.command_line.clone().unwrap_or_default()
            );
            return Ok(p);
        }
        "winget" => {
            let pkg = package()?;
            p.command = Some(vec![
                find_tool(m, "winget", &[]),
                "uninstall".into(),
                "--id".into(),
                pkg.clone(),
                "--exact".into(),
                "--silent".into(),
            ]);
            p.command_line = Some(format!("winget uninstall --id {} --exact", q(&pkg)));
        }
        "chocolatey" => {
            // choco needs an elevated shell; runa never elevates
            let pkg = package()?;
            p.command = Some(vec![
                find_tool(m, "choco", &[]),
                "uninstall".into(),
                pkg.clone(),
                "-y".into(),
            ]);
            p.command_line = Some(format!("choco uninstall {} -y", q(&pkg)));
            p.needs_root = true;
        }
        "system" | "installer" => {
            p.kind = PlanKind::Refuse;
            p.paths = vec![];
            if provider == "installer" {
                p.summary = format!(
                    "{} 由安装程序安装（MSI / 官方安装包 / winget / choco），runa 不会直接删除；请在“设置 › 应用”或用 winget / choco 卸载，然后运行 runa uninstall {tool} {version} --forget；也可以只移除管理",
                    home.display()
                );
                return Ok(p);
            }
            // system-provided runtimes are never uninstallable: hard error, not a plan
            let hint = match locations::linux_package_owner(&home) {
                Some((pm, pkg)) => {
                    let rm = if pm == "pacman" {
                        "pacman -R"
                    } else if pm == "apt" {
                        "apt remove"
                    } else {
                        "dnf remove"
                    };
                    format!("（属于系统软件包 {pkg}，如确需移除请自行执行 `sudo {rm} {pkg}`）")
                }
                None => String::new(),
            };
            bail!(
                "{tool} {version} 是系统自带的运行时（{}），禁止卸载{hint}；可以只移除管理（--forget）或禁用",
                home.display()
            );
        }
        _ => {
            let target = removal_root(&home);
            if dangerous(&target) {
                p.kind = PlanKind::Refuse;
                p.summary = format!("拒绝删除 {}：路径过于宽泛或属于系统目录", target.display());
                return Ok(p);
            }
            p.kind = PlanKind::RemoveDir;
            p.needs_root = target.parent().map(|d| !writable(d)).unwrap_or(true);
            let tq = q(&target.display().to_string());
            p.command_line = Some(if cfg!(windows) {
                format!("Remove-Item -LiteralPath {tq} -Recurse -Force")
            } else if p.needs_root {
                format!("sudo rm -rf {tq}")
            } else {
                format!("rm -rf {tq}")
            });
            p.paths = vec![target.clone()];
            p.paths.extend(runa_paths);
            p.summary = format!("删除目录 {}", target.display());
            return Ok(p);
        }
    }
    p.paths = vec![home.clone()];
    p.paths.extend(runa_paths);
    p.summary = format!(
        "通过 {provider} 彻底卸载：{}",
        p.command_line.clone().unwrap_or_default()
    );
    Ok(p)
}

fn forget(m: &Manager, plan: &UninstallPlan) -> Result<()> {
    let dir = m.paths.install_dir(&plan.tool, &plan.version);
    detect::remove_artifacts(&dir)?;
    let _ = std::fs::remove_file(installer::receipt_path(&m.paths, &plan.tool, &plan.version));
    let mut roots = vec![dir];
    roots.extend(plan.home.clone());
    m.clear_current_link_to(&plan.tool, &roots);
    Ok(())
}

/// Carry out a plan from [`plan`]. Invalidates the detection cache.
pub fn execute(m: &Manager, plan: &UninstallPlan) -> Result<()> {
    let res = execute_inner(m, plan);
    detect::invalidate_cache(m);
    res
}

fn execute_inner(m: &Manager, plan: &UninstallPlan) -> Result<()> {
    // defense in depth: a system runtime is only ever unmanaged, whatever the plan says
    if plan.kind != PlanKind::Forget
        && (plan.provider.as_deref() == Some("system")
            || plan
                .home
                .as_deref()
                .is_some_and(|h| classify(h).0 == "system"))
        && plan.kind != PlanKind::DeleteInstall
    {
        bail!(
            "{} {} 是系统自带的运行时，禁止卸载",
            plan.tool,
            plan.version
        );
    }
    match plan.kind {
        PlanKind::Refuse => bail!("{}", plan.summary),
        _ if plan.needs_root => bail!(
            "需要管理员权限，runa 不会自动使用 sudo。请在终端执行:\n  {}\n然后运行: runa uninstall {} {} --forget",
            plan.command_line.as_deref().unwrap_or(""),
            plan.tool,
            plan.version
        ),
        PlanKind::DeleteInstall => {
            let dir = m.paths.install_dir(&plan.tool, &plan.version);
            // remove_link never follows symlinks
            platform::remove_link(&dir).with_context(|| format!("删除 {} 失败", dir.display()))?;
            let _ = std::fs::remove_file(installer::receipt_path(&m.paths, &plan.tool, &plan.version));
            m.clear_current_link_to(&plan.tool, &[dir]);
            Ok(())
        }
        PlanKind::Forget => forget(m, plan),
        PlanKind::RemoveDir => {
            let target = plan.paths.first().ok_or_else(|| anyhow!("计划为空"))?;
            if dangerous(target) {
                bail!("拒绝删除 {}", target.display());
            }
            std::fs::remove_dir_all(target)
                .with_context(|| format!("删除 {} 失败", target.display()))?;
            forget(m, plan)
        }
        PlanKind::ProviderCommand => {
            let argv = plan.command.as_ref().ok_or_else(|| anyhow!("计划缺少命令"))?;
            let out = Command::new(&argv[0])
                .args(&argv[1..])
                .envs(plan.env.iter().cloned())
                .env("RUNA_DISABLE_SHIM", "1")
                .stdin(Stdio::null())
                .output()
                .with_context(|| format!("无法执行 {}", argv[0]))?;
            if !out.status.success() {
                let mut msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if msg.is_empty() {
                    msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
                bail!(
                    "{} 执行失败（{}）: {msg}",
                    plan.command_line.as_deref().unwrap_or(&argv[0]),
                    out.status
                );
            }
            if let Some(h) = &plan.home {
                if h.exists() {
                    bail!(
                        "{} 已执行，但 {} 仍然存在，保留 runa 记录",
                        plan.command_line.as_deref().unwrap_or(&argv[0]),
                        h.display()
                    );
                }
            }
            forget(m, plan)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer::{write_receipt, Receipt};
    use crate::paths::Paths;

    fn mgr() -> (tempfile::TempDir, Manager) {
        let t = tempfile::tempdir().unwrap();
        let m = Manager::with_paths(Paths::with_root(t.path().join("runa"))).unwrap();
        (t, m)
    }

    fn adopt_at(m: &Manager, tool: &str, v: &str, home: &str) {
        let (provider, formula) = classify(Path::new(home));
        write_receipt(
            &m.paths,
            &Receipt {
                tool: tool.into(),
                version: v.into(),
                source: "adopted".into(),
                home: Some(home.into()),
                origin: Some(home.into()),
                bin_dirs: vec![PathBuf::from(home).join("bin")],
                provider: Some(provider),
                formula,
                ..Default::default()
            },
        )
        .unwrap();
    }

    fn purge(m: &Manager, tool: &str, v: &str) -> UninstallPlan {
        plan(m, tool, v, Mode::Purge).unwrap()
    }

    #[test]
    fn provider_plans() {
        let (_t, m) = mgr();
        adopt_at(&m, "node", "26.8.1", "/opt/homebrew/Cellar/node/26.8.1");
        adopt_at(
            &m,
            "java",
            "17.0.9",
            "/opt/homebrew/Cellar/openjdk@17/17.0.9/libexec/openjdk.jdk/Contents/Home",
        );
        adopt_at(
            &m,
            "node",
            "24.13.0",
            "/Users/x/.nvm/versions/node/v24.13.0",
        );
        adopt_at(&m, "python", "3.12.1", "/Users/x/.pyenv/versions/3.12.1");
        adopt_at(
            &m,
            "java",
            "21.0.2",
            "/Users/x/.sdkman/candidates/java/21.0.2-tem",
        );
        adopt_at(
            &m,
            "go",
            "1.22.0",
            "/Users/x/.asdf/installs/golang/1.22.0/go",
        );
        adopt_at(
            &m,
            "node",
            "22.1.0",
            "/Users/x/Library/Application Support/fnm/node-versions/v22.1.0/installation",
        );
        adopt_at(
            &m,
            "python",
            "3.9.6",
            "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9",
        );

        let p = purge(&m, "node", "26.8.1");
        assert_eq!(p.kind, PlanKind::ProviderCommand);
        assert_eq!(p.command_line.as_deref(), Some("brew uninstall node"));
        assert!(p
            .command
            .as_ref()
            .unwrap()
            .ends_with(&["uninstall".into(), "node".into()]));
        assert_eq!(
            purge(&m, "java", "17.0.9").command_line.as_deref(),
            Some("brew uninstall openjdk@17")
        );

        let p = purge(&m, "node", "24.13.0");
        let line = p.command_line.unwrap();
        assert!(line.contains("NVM_DIR=/Users/x/.nvm;"), "{line}");
        assert!(line.contains("nvm uninstall v24.13.0"), "{line}");

        let p = purge(&m, "python", "3.12.1");
        assert_eq!(p.command_line.as_deref(), Some("pyenv uninstall -f 3.12.1"));
        assert!(p
            .env
            .contains(&("PYENV_ROOT".into(), "/Users/x/.pyenv".into())));

        let line = purge(&m, "java", "21.0.2").command_line.unwrap();
        assert!(
            line.contains("SDKMAN_DIR=/Users/x/.sdkman;")
                && line.contains("sdk uninstall java 21.0.2-tem"),
            "{line}"
        );

        let p = purge(&m, "go", "1.22.0");
        assert_eq!(
            p.command_line.as_deref(),
            Some("asdf uninstall golang 1.22.0")
        );
        assert!(p
            .env
            .contains(&("ASDF_DATA_DIR".into(), "/Users/x/.asdf".into())));

        let p = purge(&m, "node", "22.1.0");
        assert_eq!(p.command_line.as_deref(), Some("fnm uninstall v22.1.0"));
        assert!(p.env.contains(&(
            "FNM_DIR".into(),
            "/Users/x/Library/Application Support/fnm".into()
        )));

        let e = plan(&m, "python", "3.9.6", Mode::Purge).unwrap_err();
        assert!(format!("{e:#}").contains("禁止卸载"), "{e:#}");
        // forget is always possible and keeps the files
        let f = plan(&m, "python", "3.9.6", Mode::Forget).unwrap();
        assert_eq!(f.kind, PlanKind::Forget);
        assert!(f.paths.iter().all(|p| p.starts_with(&m.paths.root)));
        // adopted versions need an explicit choice
        assert!(plan(&m, "node", "26.8.1", Mode::Auto).is_err());
    }

    /// Windows and Linux owners plan the owning tool's command (path layouts only, so it
    /// runs on any host; nothing is executed).
    #[test]
    fn windows_and_linux_provider_plans() {
        let (_t, m) = mgr();
        let cases: &[(&str, &str, &str)] = &[
            (
                "node",
                "20.11.1",
                r"C:\Users\a\AppData\Roaming\nvm\v20.11.1",
            ),
            (
                "python",
                "3.12.1",
                r"C:\Users\a\.pyenv\pyenv-win\versions\3.12.1",
            ),
            ("node", "21.6.0", r"C:\Users\a\scoop\apps\nodejs\21.6.0"),
            (
                "python",
                "3.11.0",
                r"C:\ProgramData\scoop\apps\python\3.11.0",
            ),
            (
                "go",
                "1.22.0",
                r"C:\Users\a\AppData\Local\Microsoft\WinGet\Packages\GoLang.Go_Microsoft.Winget.Source_8wekyb3d8bbwe\go",
            ),
            (
                "go",
                "1.21.0",
                r"C:\ProgramData\chocolatey\lib\golang\tools\go",
            ),
            (
                "java",
                "21.0.2",
                r"C:\Program Files\Eclipse Adoptium\jdk-21.0.2.13-hotspot",
            ),
            (
                "python",
                "3.10.0",
                r"C:\Users\a\AppData\Local\Programs\Python\Python310",
            ),
            (
                "node",
                "18.0.0",
                "/home/u/.local/share/mise/installs/node/18.0.0",
            ),
            ("java", "17.0.10", "/usr/lib/jvm/java-17-openjdk-amd64"),
        ];
        for (tool, v, home) in cases {
            adopt_at(&m, tool, v, home);
        }
        let line = |tool: &str, v: &str| purge(&m, tool, v).command_line.unwrap_or_default();
        let p = purge(&m, "node", "20.11.1");
        assert_eq!(p.provider.as_deref(), Some("nvm-windows"));
        assert_eq!(p.command_line.as_deref(), Some("nvm uninstall 20.11.1"));
        assert!(p.env.iter().any(|(k, _)| k == "NVM_HOME"));
        assert_eq!(line("python", "3.12.1"), "pyenv uninstall 3.12.1");
        let p = purge(&m, "node", "21.6.0");
        assert_eq!(p.command_line.as_deref(), Some("scoop uninstall nodejs"));
        assert!(!p.needs_root);
        let p = purge(&m, "python", "3.11.0");
        assert_eq!(p.command_line.as_deref(), Some("scoop uninstall -g python"));
        assert!(
            p.needs_root && execute(&m, &p).is_err(),
            "runa never elevates"
        );
        assert_eq!(
            line("go", "1.22.0"),
            "winget uninstall --id GoLang.Go --exact"
        );
        let p = purge(&m, "go", "1.21.0");
        assert_eq!(p.command_line.as_deref(), Some("choco uninstall golang -y"));
        assert!(p.needs_root);
        for (t, v) in [("java", "21.0.2"), ("python", "3.10.0")] {
            let p = purge(&m, t, v);
            assert_eq!(p.kind, PlanKind::Refuse, "{t} {v}");
            assert_eq!(p.provider.as_deref(), Some("installer"));
            assert!(p.paths.is_empty());
            // unmanaging is always possible
            assert_eq!(plan(&m, t, v, Mode::Forget).unwrap().kind, PlanKind::Forget);
        }
        assert_eq!(line("node", "18.0.0"), "mise uninstall node@18.0.0");
        assert!(plan(&m, "java", "17.0.10", Mode::Purge).is_err());
    }

    /// System runtimes on every platform: purge is an error, a forged purge plan is
    /// refused by execute, unmanaging works and keeps the files.
    #[test]
    fn system_runtimes_are_never_uninstallable() {
        let (_t, m) = mgr();
        let homes = [
            "/usr",
            "/usr/lib/jvm/java-21-openjdk-amd64",
            "/System/Library/Frameworks/Python.framework/Versions/2.7",
            "/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9",
            r"C:\Windows\py",
            r"C:\Users\a\AppData\Local\Microsoft\WindowsApps",
        ];
        for (i, h) in homes.iter().enumerate() {
            let v = format!("1.0.{i}");
            adopt_at(&m, "python", &v, h);
            let e = plan(&m, "python", &v, Mode::Purge).unwrap_err();
            assert!(format!("{e:#}").contains("禁止卸载"), "{h}: {e:#}");
            let mut forged = plan(&m, "python", &v, Mode::Forget).unwrap();
            forged.kind = PlanKind::RemoveDir;
            forged.paths = vec![PathBuf::from(h)];
            assert!(execute(&m, &forged).is_err(), "{h}");
            let f = plan(&m, "python", &v, Mode::Forget).unwrap();
            assert_eq!(f.kind, PlanKind::Forget);
            assert!(f.paths.iter().all(|p| p.starts_with(&m.paths.root)));
            execute(&m, &f).unwrap();
            assert!(!m.is_installed("python", &v));
        }
    }

    #[test]
    fn manual_plan_removes_whole_bundle_and_executes_in_temp() {
        let (t, m) = mgr();
        let bundle = t.path().join("jvms").join("jbr-21.0.11");
        let home = bundle.join("Contents").join("Home");
        std::fs::create_dir_all(home.join("bin")).unwrap();
        adopt_at(&m, "java", "21.0.11", home.to_str().unwrap());
        let p = purge(&m, "java", "21.0.11");
        assert_eq!(p.kind, PlanKind::RemoveDir);
        assert_eq!(p.paths[0], platform::canonicalize(&bundle));
        assert!(!p.needs_root);
        execute(&m, &p).unwrap();
        assert!(!bundle.exists());
        assert!(m.receipt("java", "21.0.11").is_none());
        assert!(m.installed("java").is_empty());
    }

    #[test]
    fn root_owned_dirs_are_not_sudoed() {
        if !Path::new("/Library/Java").exists() && !Path::new("/usr/lib/jvm").exists() {
            return;
        }
        let (_t, m) = mgr();
        let home = if cfg!(target_os = "macos") {
            "/Library/Java/JavaVirtualMachines/fake-runa-test.jdk/Contents/Home"
        } else {
            "/opt/fake-runa-test-jdk"
        };
        adopt_at(&m, "java", "1.0.0", home);
        let p = purge(&m, "java", "1.0.0");
        if p.needs_root {
            assert!(p
                .command_line
                .as_deref()
                .unwrap()
                .starts_with("sudo rm -rf "));
            let e = execute(&m, &p).unwrap_err().to_string();
            assert!(e.contains("sudo"), "{e}");
        }
    }

    #[test]
    fn download_plan_and_forget_rejected() {
        let (_t, m) = mgr();
        std::fs::create_dir_all(m.paths.install_dir("go", "1.22.0").join("bin")).unwrap();
        let p = plan(&m, "go", "1.22.0", Mode::Auto).unwrap();
        assert_eq!(p.kind, PlanKind::DeleteInstall);
        assert!(plan(&m, "go", "1.22.0", Mode::Forget).is_err());
        execute(&m, &p).unwrap();
        assert!(!m.is_installed("go", "1.22.0"));
    }

    #[cfg(unix)]
    #[test]
    fn legacy_origin_bin_dir_is_never_the_purge_target() {
        let (t, m) = mgr();
        // old style node adoption: private bin/ with links into a shared bin dir
        let shared = std::fs::canonicalize(t.path()).unwrap().join("shared-bin");
        std::fs::create_dir_all(&shared).unwrap();
        let nd = m.paths.install_dir("node", "20.0.0");
        std::fs::create_dir_all(nd.join("bin")).unwrap();
        std::os::unix::fs::symlink(shared.join("node"), nd.join("bin/node")).unwrap();
        write_receipt(
            &m.paths,
            &Receipt {
                tool: "node".into(),
                version: "20.0.0".into(),
                source: "adopted".into(),
                origin: Some(shared.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        // nothing real behind it: purge must refuse instead of deleting the shared dir
        assert!(plan(&m, "node", "20.0.0", Mode::Purge).is_err());
        let f = plan(&m, "node", "20.0.0", Mode::Forget).unwrap();
        assert!(f.paths.iter().all(|p| p.starts_with(&m.paths.root)));
        execute(&m, &f).unwrap();
        assert!(shared.is_dir());
        assert!(!m.is_installed("node", "20.0.0"));
    }

    #[test]
    fn removal_root_handles_bundles_and_java8_jre() {
        let b = Path::new("/u/Library/Java/JavaVirtualMachines/corretto-1.8.0_482");
        assert_eq!(removal_root(&b.join("Contents/Home")), b);
        assert_eq!(removal_root(&b.join("Contents/Home/jre")), b);
        assert_eq!(
            removal_root(Path::new("/u/sdk/go1.22")),
            Path::new("/u/sdk/go1.22")
        );
    }

    #[test]
    fn quoting() {
        assert_eq!(q_sh("node"), "node");
        assert_eq!(q_sh("a b"), "'a b'");
        assert_eq!(q_sh("it's"), "'it'\\''s'");
        assert_eq!(q_powershell(r"C:\Program Files\x"), r"'C:\Program Files\x'");
        assert_eq!(q_powershell("it's"), "'it''s'");
        assert!(safe_id("v1.2.3").is_ok());
        assert!(safe_id("1;rm").is_err());
        assert!(safe_id("-rf").is_err());
    }
}
