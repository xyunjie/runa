//! Detect runtimes already installed on the machine and adopt them (symlinks only, the
//! original installation is never modified or moved).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
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
    /// Already adopted into runa.
    pub adopted: bool,
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
    v
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

/// Scan PATH, tool env vars and well-known directories.
pub fn detect(m: &Manager) -> Vec<Detected> {
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
                let adopted = m.installed(&tool).iter().any(|v| {
                    m.receipt(&tool, v)
                        .and_then(|r| r.origin)
                        .map(|o| o == home || o == dir || o == bin_dir)
                        .unwrap_or(false)
                });
                found.push(Detected {
                    tool: tool.clone(),
                    version,
                    exe: p,
                    found_in: bin_dir,
                    home,
                    via: via.clone(),
                    adopted,
                });
                break; // python3 found -> skip python in the same dir
            }
        }
    }
    found
}

/// Ask the runtime itself for its installation root. Works through stubs and shims.
fn query_home(tool: &str, exe: &Path) -> Option<PathBuf> {
    let (args, key): (&[&str], Option<&str>) = match tool {
        "java" => (&["-XshowSettings:properties", "-version"], Some("java.home = ")),
        "python" => (&["-c", "import sys; print(sys.base_prefix)"], None),
        "node" => (&["-p", "process.execPath"], None),
        "go" => (&["env", "GOROOT"], None),
        _ => return None,
    };
    let out = run_version(exe, args)?;
    let line = match key {
        Some(k) => out.lines().find_map(|l| l.trim().strip_prefix(k))?.trim().to_string(),
        None => out.lines().next()?.trim().to_string(),
    };
    if line.is_empty() {
        return None;
    }
    let mut p = PathBuf::from(line);
    if tool == "node" {
        // execPath is the binary: <root>/bin/node
        p = std::fs::canonicalize(&p).ok()?.parent()?.parent()?.to_path_buf();
    }
    let p = std::fs::canonicalize(&p).unwrap_or(p);
    p.is_dir().then_some(p)
}

/// Fallback: `<root>/bin/<exe>`, accepted only when that layout really exists, so a
/// stub such as /usr/bin/java never turns /usr into a "JDK".
fn guess_home(real: &Path, exe: &str) -> Option<PathBuf> {
    let home = real.parent()?.parent()?.to_path_buf();
    if is_system_prefix(&home) {
        return None;
    }
    real_bin_dir(&home, exe).map(|_| home)
}

fn real_bin_dir(home: &Path, exe: &str) -> Option<PathBuf> {
    [home.join("bin"), home.to_path_buf()]
        .into_iter()
        .find(|d| crate::manager::find_exe_in(std::slice::from_ref(d), exe).is_some())
}

/// Shared system prefixes are never a runtime root by themselves.
fn is_system_prefix(p: &Path) -> bool {
    matches!(
        p.to_str(),
        Some("/" | "/usr" | "/usr/local" | "/opt/homebrew" | "/opt/local")
    )
}

/// Adopt a detected runtime as `installs/<tool>/<version>`.
///
/// - Self-contained runtimes (JDK, Go): the version dir is a symlink to the original root.
/// - Runtimes in shared dirs (e.g. node in /usr/bin): a private `bin/` with symlinks to the
///   relevant executables only, so adopting does not leak unrelated programs onto PATH.
/// - If symlinks are not permitted (Windows without Developer Mode) the receipt records the
///   original paths instead.
pub fn adopt(m: &Manager, d: &Detected, name: Option<&str>) -> Result<String> {
    let plugin = m.plugin(&d.tool)?;
    let version = name.unwrap_or(&d.version).to_string();
    installer::validate_version_name(&version)?;
    let dest = m.paths.install_dir(&d.tool, &version);
    if std::fs::symlink_metadata(&dest).is_ok() {
        bail!(
            "{} {} 已存在，可用 --name 指定其它名称（如 {}-system）",
            d.tool,
            version,
            version
        );
    }
    std::fs::create_dir_all(m.paths.tool_dir(&d.tool))?;
    let mut receipt = Receipt {
        tool: d.tool.clone(),
        version: version.clone(),
        source: "adopted".into(),
        installed_at: installer::now(),
        ..Default::default()
    };
    match plugin.adopt_exes() {
        None => {
            receipt.origin = Some(d.home.clone());
            if platform::symlink_dir(&d.home, &dest).is_err() {
                std::fs::create_dir_all(&dest)?;
                receipt.home = Some(d.home.clone());
            }
        }
        Some(names) => {
            receipt.origin = Some(d.found_in.clone());
            let bin = dest.join("bin");
            std::fs::create_dir_all(&bin)?;
            let mut linked = 0;
            let mut symlink_failed = false;
            let mut wanted: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            if d.tool == "python" {
                let mm: Vec<&str> = d.version.split('.').take(2).collect();
                if mm.len() == 2 {
                    wanted.push(format!("python{}.{}", mm[0], mm[1]));
                    wanted.push(format!("pip{}.{}", mm[0], mm[1]));
                }
            }
            for n in wanted {
                if let Some(src) =
                    crate::manager::find_exe_in(std::slice::from_ref(&d.found_in), &n)
                {
                    let link = bin.join(src.file_name().unwrap());
                    match platform::symlink_file(&src, &link) {
                        Ok(_) => linked += 1,
                        Err(_) => symlink_failed = true,
                    }
                }
            }
            if symlink_failed || linked == 0 {
                receipt.bin_dirs = vec![d.found_in.clone()];
            }
        }
    }
    installer::write_receipt(&m.paths, &receipt).context("写入安装记录失败")?;
    Ok(version)
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
}
