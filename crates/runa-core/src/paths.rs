use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

/// Layout of the runa home directory (default `~/.runa`, override with `RUNA_HOME`).
///
/// ```text
/// ~/.runa/
///   .tool-versions        global versions (asdf compatible)
///   settings.json         mirror / proxy settings
///   installs/<tool>/<version>/
///   shims/                generated shims
///   current/<tool>        symlink to the global version (best effort)
///   plugins/*.json        declarative manifest plugins
///   cache/downloads/      downloaded archives
///   cache/detect.json     detection cache
/// ```
#[derive(Debug, Clone)]
pub struct Paths {
    /// Where installs, shims and `.tool-versions` live. For an isolated project this is
    /// `<project>/.runa/env`; otherwise it equals `home`.
    pub root: PathBuf,
    /// The user's runa home: settings and JSON plugins are always read from here.
    pub home: PathBuf,
}

impl Paths {
    pub fn detect() -> Result<Self> {
        if let Some(v) = std::env::var_os("RUNA_HOME") {
            if !v.is_empty() {
                return Ok(Self::with_root(PathBuf::from(v)));
            }
        }
        let home = dirs::home_dir().ok_or_else(|| anyhow!("无法确定用户主目录"))?;
        Ok(Self::with_root(home.join(".runa")))
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            home: root.clone(),
            root,
        }
    }

    /// Same home, but installs / shims / versions under `root` (isolated project).
    pub fn isolated(&self, root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            home: self.home.clone(),
        }
    }

    pub fn is_isolated(&self) -> bool {
        self.root != self.home
    }

    pub fn installs(&self) -> PathBuf {
        self.root.join("installs")
    }
    pub fn tool_dir(&self, tool: &str) -> PathBuf {
        self.installs().join(tool)
    }
    pub fn install_dir(&self, tool: &str, version: &str) -> PathBuf {
        self.tool_dir(tool).join(version)
    }
    pub fn shims(&self) -> PathBuf {
        self.root.join("shims")
    }
    pub fn current(&self) -> PathBuf {
        self.root.join("current")
    }
    pub fn plugins(&self) -> PathBuf {
        self.home.join("plugins")
    }
    pub fn downloads(&self) -> PathBuf {
        self.root.join("cache").join("downloads")
    }
    /// Cached result of `runa detect` (shared by the CLI and the GUI).
    pub fn detect_cache(&self) -> PathBuf {
        self.home.join("cache").join("detect.json")
    }
    pub fn settings_file(&self) -> PathBuf {
        self.home.join("settings.json")
    }
    pub fn global_tool_versions(&self) -> PathBuf {
        self.root.join(".tool-versions")
    }

    pub fn ensure(&self) -> Result<()> {
        for d in [
            self.installs(),
            self.shims(),
            self.plugins(),
            self.downloads(),
            self.current(),
        ] {
            std::fs::create_dir_all(&d)?;
        }
        Ok(())
    }

    /// True when `p` lives inside the runa home (used to skip our own shims in detection).
    pub fn contains(&self, p: &Path) -> bool {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        [&self.root, &self.home].iter().any(|r| {
            let r = std::fs::canonicalize(r).unwrap_or_else(|_| r.to_path_buf());
            p.starts_with(&r)
        })
    }
}
