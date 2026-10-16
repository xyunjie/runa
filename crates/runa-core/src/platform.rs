use std::path::{Path, PathBuf};

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

/// `std::fs::canonicalize` without the `\\?\` verbatim prefix Windows adds (UNC paths
/// are kept verbatim), falling back to the path itself when it does not exist.
pub fn canonicalize(p: &Path) -> PathBuf {
    strip_verbatim(std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
}

/// Remove a `\\?\C:\` verbatim prefix (no-op elsewhere).
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC") {
            return PathBuf::from(rest.to_string());
        }
    }
    p
}

/// Comparison key for a path: `/` separators, no verbatim prefix or trailing separator,
/// and ASCII lower-case when the platform's file system is case-insensitive (Windows).
pub fn path_key(p: &Path) -> String {
    path_key_for(p, cfg!(windows))
}

/// [`path_key`] with explicit case folding (testable on any platform).
pub fn path_key_for(p: &Path, fold_case: bool) -> String {
    let s = p.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('\\', "/");
    let mut s = if fold_case { s.to_ascii_lowercase() } else { s };
    while s.len() > 1 && s.ends_with('/') && !s.ends_with(":/") {
        s.pop();
    }
    s
}

/// Same path on this platform (case-insensitive on Windows; separators normalized).
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_key(a) == path_key(b)
}

/// `p` is `root` or inside it, using [`path_key`] semantics.
pub fn path_within(p: &Path, root: &Path) -> bool {
    let (p, r) = (path_key(p), path_key(root));
    p == r || (p.starts_with(&r) && (r.ends_with('/') || p[r.len()..].starts_with('/')))
}

/// Executable extensions tried for a bare command name on Windows: `PATHEXT` (only the
/// ones `CreateProcess` can start directly) or `.exe .cmd .bat .com`.
pub fn exe_extensions() -> Vec<String> {
    let fallback = || vec![".exe".into(), ".cmd".into(), ".bat".into(), ".com".into()];
    let Some(v) = std::env::var_os("PATHEXT") else {
        return fallback();
    };
    let exts: Vec<String> = v
        .to_string_lossy()
        .split(';')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), ".exe" | ".cmd" | ".bat" | ".com"))
        .collect();
    if exts.is_empty() {
        fallback()
    } else {
        exts
    }
}

/// File names to look for when searching `exe` in a directory (`node` -> `node.exe`,
/// `node.cmd`, ... on Windows; unchanged elsewhere or when it already has an extension).
pub fn exe_candidates(exe: &str) -> Vec<String> {
    if cfg!(windows) && Path::new(exe).extension().is_none() {
        exe_extensions()
            .iter()
            .map(|e| format!("{exe}{e}"))
            .collect()
    } else {
        vec![exe.to_string()]
    }
}

/// Command name without a Windows executable / script extension (`java.exe` -> `java`,
/// `npm.cmd` -> `npm`), case-insensitively; other names are returned unchanged.
pub fn bare_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
        if lower.len() > ext.len() && lower.ends_with(ext) {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_keys_and_containment() {
        let k = |s: &str| path_key_for(Path::new(s), true);
        assert_eq!(k(r"\\?\C:\Program Files\Go\"), "c:/program files/go");
        assert_eq!(k("C:/"), "c:/");
        assert_eq!(path_key_for(Path::new("/Users/A/"), false), "/Users/A");
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\x")),
            PathBuf::from(r"C:\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\srv\s")),
            PathBuf::from(r"\\?\UNC\srv\s")
        );
        let root = Path::new("/a").join("b");
        assert!(path_within(&root.join("c"), &root));
        assert!(path_within(&root, &root));
        assert!(!path_within(Path::new("/a/bc"), &root));
    }

    #[test]
    fn bare_names() {
        assert_eq!(bare_name("java.exe"), "java");
        assert_eq!(bare_name("NPM.CMD"), "NPM");
        assert_eq!(bare_name("python3"), "python3");
        assert_eq!(bare_name(".exe"), ".exe");
    }

    #[cfg(windows)]
    #[test]
    fn windows_candidates_and_case() {
        assert!(exe_candidates("node").iter().any(|n| n == "node.exe"));
        assert_eq!(exe_candidates("node.exe"), vec!["node.exe".to_string()]);
        assert!(same_path(Path::new(r"C:\Go"), Path::new("c:/go/")));
    }
}
