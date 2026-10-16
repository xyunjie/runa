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

use crate::detect::{self, classify, segments_after};
use crate::installer;
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

fn q(s: &str) -> String {
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

fn dangerous(p: &Path) -> bool {
    p.components().count() < 3
        || detect::is_system_prefix(p)
        || dirs::home_dir()
            .map(|h| h == p || h.starts_with(p))
            .unwrap_or(false)
        || classify(p).0 == "system"
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
    Some(std::fs::canonicalize(&h).unwrap_or(h))
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
    let (cls_provider, cls_formula) = home
        .as_deref()
        .map(classify)
        .unwrap_or_else(|| ("manual".into(), None));
    let provider = r.provider.clone().unwrap_or(cls_provider);
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
    let hs = home.to_string_lossy().replace('\\', "/");
    let user_home = dirs::home_dir().unwrap_or_default();
    let prefix_of = |marker: &str| -> PathBuf {
        // "/Users/x/.nvm/versions/..." with marker "/.nvm/" -> "/Users/x/.nvm"
        PathBuf::from(
            &hs[..hs
                .find(marker)
                .map(|i| i + marker.len() - 1)
                .unwrap_or(hs.len())],
        )
    };
    p.kind = PlanKind::ProviderCommand;
    match provider.as_str() {
        "homebrew" => {
            let formula = r
                .formula
                .clone()
                .or(cls_formula)
                .ok_or_else(|| anyhow!("无法确定 Homebrew formula"))?;
            safe_id(&formula)?;
            p.command = Some(vec![
                find_tool(m, "brew", &[]),
                "uninstall".into(),
                formula.clone(),
            ]);
            p.command_line = Some(format!("brew uninstall {}", q(&formula)));
        }
        "nvm" => {
            let id = segments_after(&hs, "/.nvm/versions/node/")
                .first()
                .copied()
                .unwrap_or("");
            let id = safe_id(id)?;
            let nvm_dir = prefix_of("/.nvm/");
            let script = format!(
                "export NVM_DIR={}; source \"$NVM_DIR/nvm.sh\" && nvm uninstall {}",
                q(&nvm_dir.display().to_string()),
                q(id)
            );
            p.command = Some(vec!["bash".into(), "-c".into(), script.clone()]);
            p.command_line = Some(format!("bash -c {}", q(&script)));
        }
        "pyenv" => {
            let id = safe_id(
                segments_after(&hs, "/.pyenv/versions/")
                    .first()
                    .copied()
                    .unwrap_or(""),
            )?;
            let root = prefix_of("/.pyenv/");
            let exe = find_tool(m, "pyenv", &[root.join("bin")]);
            p.env
                .push(("PYENV_ROOT".into(), root.display().to_string()));
            p.command = Some(vec![exe, "uninstall".into(), "-f".into(), id.into()]);
            p.command_line = Some(format!("pyenv uninstall -f {}", q(id)));
        }
        "sdkman" => {
            let seg = segments_after(&hs, "/.sdkman/candidates/");
            let cand = safe_id(seg.first().copied().unwrap_or(""))?;
            let id = safe_id(seg.get(1).copied().unwrap_or(""))?;
            let sd = prefix_of("/.sdkman/");
            let script = format!(
                "export SDKMAN_DIR={}; source \"$SDKMAN_DIR/bin/sdkman-init.sh\" && sdk uninstall {} {}",
                q(&sd.display().to_string()),
                q(cand),
                q(id)
            );
            p.command = Some(vec!["bash".into(), "-c".into(), script.clone()]);
            p.command_line = Some(format!("bash -c {}", q(&script)));
        }
        "asdf" => {
            let seg = segments_after(&hs, "/.asdf/installs/");
            let plugin = safe_id(seg.first().copied().unwrap_or(""))?;
            let id = safe_id(seg.get(1).copied().unwrap_or(""))?;
            let data = prefix_of("/.asdf/");
            let exe = find_tool(m, "asdf", &[data.join("bin")]);
            p.env
                .push(("ASDF_DATA_DIR".into(), data.display().to_string()));
            p.command = Some(vec![exe, "uninstall".into(), plugin.into(), id.into()]);
            p.command_line = Some(format!("asdf uninstall {} {}", q(plugin), q(id)));
        }
        "fnm" => {
            let id = safe_id(
                segments_after(&hs, "/fnm/node-versions/")
                    .first()
                    .copied()
                    .unwrap_or(""),
            )?;
            let fnm_dir = PathBuf::from(&hs[..hs.find("/node-versions/").unwrap_or(hs.len())]);
            let exe = find_tool(
                m,
                "fnm",
                &[
                    user_home.join(".local/share/fnm"),
                    user_home.join(".cargo/bin"),
                ],
            );
            p.env
                .push(("FNM_DIR".into(), fnm_dir.display().to_string()));
            p.command = Some(vec![exe, "uninstall".into(), id.into()]);
            p.command_line = Some(format!("fnm uninstall {}", q(id)));
        }
        "system" => {
            p.kind = PlanKind::Refuse;
            p.summary = format!(
                "{} 是系统自带的运行时（由 macOS / Xcode / 系统包管理器管理），runa 不能卸载；可以只移除管理",
                home.display()
            );
            p.paths = vec![];
            return Ok(p);
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
            p.command_line = Some(if p.needs_root {
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

        let p = purge(&m, "python", "3.9.6");
        assert_eq!(p.kind, PlanKind::Refuse);
        assert!(execute(&m, &p).is_err());
        // forget is always possible and keeps the files
        let f = plan(&m, "python", "3.9.6", Mode::Forget).unwrap();
        assert_eq!(f.kind, PlanKind::Forget);
        assert!(f.paths.iter().all(|p| p.starts_with(&m.paths.root)));
        // adopted versions need an explicit choice
        assert!(plan(&m, "node", "26.8.1", Mode::Auto).is_err());
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
        assert_eq!(p.paths[0], std::fs::canonicalize(&bundle).unwrap());
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
        assert_eq!(q("node"), "node");
        assert_eq!(q("a b"), "'a b'");
        assert_eq!(q("it's"), "'it'\\''s'");
        assert!(safe_id("v1.2.3").is_ok());
        assert!(safe_id("1;rm").is_err());
        assert!(safe_id("-rf").is_err());
    }
}
