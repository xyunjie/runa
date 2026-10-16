//! Language plugins. Built-in plugins implement [`Plugin`] in Rust; extra tools can be
//! described declaratively with JSON manifests (see [`manifest`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::net::Net;
use crate::platform::Target;
use crate::version::VersionReq;

pub mod bun;
pub mod deno;
pub mod go;
pub mod java;
pub mod manifest;
pub mod node;
pub mod python;
pub mod ruby;
pub mod rust;
mod util;

/// A remote version with metadata for UIs.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RemoteVersion {
    pub version: String,
    pub lts: bool,
    pub prerelease: bool,
}

/// A downloadable archive for one version on one platform.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub url: String,
    pub file_name: String,
    /// Upstream SHA-256, when published.
    pub sha256: Option<String>,
}

pub trait Plugin: Send + Sync {
    /// Tool name as used in `.tool-versions` (e.g. `node`).
    fn name(&self) -> &str;
    /// Human readable description (Chinese UI text).
    fn description(&self) -> String {
        self.name().to_string()
    }
    /// All installable versions, newest first.
    fn list_all(&self, net: &Net) -> Result<Vec<String>>;
    /// Installable versions with LTS / pre-release flags, newest first.
    fn list_remote(&self, net: &Net) -> Result<Vec<RemoteVersion>> {
        Ok(self
            .list_all(net)?
            .into_iter()
            .map(|v| RemoteVersion {
                prerelease: crate::version::is_prerelease(&v),
                lts: false,
                version: v,
            })
            .collect())
    }
    /// Turn a (possibly fuzzy) requirement into a concrete installable version.
    fn resolve_remote(&self, net: &Net, req: &VersionReq) -> Result<String> {
        let all = self.list_all(net)?;
        let req = if *req == VersionReq::Lts {
            VersionReq::Latest
        } else {
            req.clone()
        };
        req.best(all.iter())
            .ok_or_else(|| anyhow::anyhow!("{} 没有匹配 {:?} 的可安装版本", self.name(), req))
    }
    /// Where to download `version` for `target`.
    fn artifact(&self, net: &Net, version: &str, target: &Target) -> Result<Artifact>;
    /// Whether runa can download this tool itself on `target` (Ruby: never, there is no
    /// portable official binary; Rust on Windows: only through rustup).
    fn supports_download(&self, _target: &Target) -> bool {
        true
    }
    /// Turn an extracted archive into the final install layout (runs in the staging dir).
    fn post_extract(&self, _staged: &Path) -> Result<()> {
        Ok(())
    }
    /// Directories to put on PATH for an install.
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        vec![install_dir.join("bin")]
    }
    /// Extra environment variables (JAVA_HOME, GOROOT...).
    fn env_vars(&self, _install_dir: &Path) -> Vec<(String, String)> {
        vec![]
    }
    /// Executables used to detect a system installation (first found wins).
    fn detect_exes(&self) -> Vec<&'static str> {
        vec![]
    }
    /// Arguments printing the version.
    fn version_args(&self) -> Vec<&'static str> {
        vec!["--version"]
    }
    /// Parse the output of the version command.
    fn parse_version_output(&self, out: &str) -> Option<String> {
        first_version(out)
    }
    /// Executables to link when adopting from a shared bin dir (e.g. /usr/bin).
    /// `None` means the whole bin dir belongs to the runtime (JDK, Go).
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        None
    }
    /// True for plugins compiled into runa.
    fn builtin(&self) -> bool {
        true
    }
}

/// First `\d+(\.\d+)+` sequence in a string.
pub fn first_version(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let cand = s[start..i].trim_end_matches('.');
            if cand.contains('.') {
                return Some(cand.to_string());
            }
        }
        i += 1;
    }
    None
}

pub type PluginRef = Arc<dyn Plugin>;

#[derive(Clone)]
pub struct Registry {
    plugins: Vec<PluginRef>,
}

impl Registry {
    /// Built-in plugins plus every `*.json` manifest in `plugins_dir`.
    pub fn load(plugins_dir: &Path) -> (Self, Vec<String>) {
        let mut plugins: Vec<PluginRef> = vec![
            Arc::new(node::Node),
            Arc::new(java::Java),
            Arc::new(python::Python),
            Arc::new(go::Go),
            Arc::new(rust::Rust),
            Arc::new(ruby::Ruby),
            Arc::new(bun::Bun),
            Arc::new(deno::Deno),
        ];
        let mut warnings = Vec::new();
        for (m, w) in manifest::load_dir(plugins_dir) {
            if let Some(w) = w {
                warnings.push(w);
            }
            if let Some(m) = m {
                if plugins.iter().any(|p| p.name() == m.name()) {
                    warnings.push(format!("插件 {} 与内置插件重名，已忽略", m.name()));
                    continue;
                }
                plugins.push(Arc::new(m));
            }
        }
        (Self { plugins }, warnings)
    }

    pub fn get(&self, name: &str) -> Option<PluginRef> {
        let name = crate::toolversions::canonical_tool(name);
        self.plugins.iter().find(|p| p.name() == name).cloned()
    }

    pub fn all(&self) -> &[PluginRef] {
        &self.plugins
    }

    pub fn names(&self) -> Vec<String> {
        self.plugins.iter().map(|p| p.name().to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_output() {
        assert_eq!(first_version("v20.11.1\n").as_deref(), Some("20.11.1"));
        assert_eq!(first_version("Python 3.12.1").as_deref(), Some("3.12.1"));
        assert_eq!(
            first_version("go version go1.22.1 linux/amd64").as_deref(),
            Some("1.22.1")
        );
        assert_eq!(
            first_version("openjdk version \"21.0.2\" 2024-01-16").as_deref(),
            Some("21.0.2")
        );
        assert_eq!(first_version("nothing 1 here"), None);
    }
}
