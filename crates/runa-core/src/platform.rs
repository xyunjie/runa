use std::path::Path;

/// Target platform in runa's naming: os = linux | macos | windows, arch = x64 | arm64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub os: String,
    pub arch: String,
}

impl Target {
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "macos" => "macos",
            "windows" => "windows",
            _ => "linux",
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            "x86" => "x86",
            _ => "x64",
        };
        Self {
            os: os.into(),
            arch: arch.into(),
        }
    }

    pub fn new(os: &str, arch: &str) -> Self {
        Self {
            os: os.into(),
            arch: arch.into(),
        }
    }

    pub fn is_windows(&self) -> bool {
        self.os == "windows"
    }

    /// Key used in manifest asset maps, e.g. `linux/x64`.
    pub fn key(&self) -> String {
        format!("{}/{}", self.os, self.arch)
    }
}

/// Executable file name for the current platform (`node` -> `node.exe` on Windows).
pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Whether a file looks executable on this platform.
pub fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        matches!(ext.as_str(), "exe" | "cmd" | "bat" | "com" | "ps1")
    }
}

/// Command name with platform extensions stripped (`npm.cmd` -> `npm`).
pub fn command_stem(p: &Path) -> Option<String> {
    let name = p.file_name()?.to_str()?.to_string();
    if cfg!(windows) {
        let lower = name.to_ascii_lowercase();
        for ext in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
            if lower.ends_with(ext) {
                return Some(name[..name.len() - ext.len()].to_string());
            }
        }
        return None;
    }
    Some(name)
}

/// Create a directory symlink (best effort on Windows, where it may need Developer Mode).
pub fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
}

/// Create a file symlink.
pub fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// Remove a file or a symlink (to file or directory) without following it.
pub fn remove_link(p: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.file_type().is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p).or_else(|_| std::fs::remove_dir(p)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
