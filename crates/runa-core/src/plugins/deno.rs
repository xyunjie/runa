use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::util::{first_sha256, rust_triple, versions_from_headings};
use super::{Artifact, Plugin, RemoteVersion};
use crate::net::Net;
use crate::platform::Target;

const REL: &str = "https://github.com/denoland/deno/releases/download/";
const NOTES: &str = "https://raw.githubusercontent.com/denoland/deno/main/Releases.md";

/// Deno from its GitHub releases (each asset has a `.sha256sum` next to it).
pub struct Deno;

impl Plugin for Deno {
    fn name(&self) -> &str {
        "deno"
    }
    fn description(&self) -> String {
        "Deno JavaScript / TypeScript 运行时（GitHub 官方发行版）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let v = versions_from_headings(&net.get_text(NOTES)?, "### ");
        if v.is_empty() {
            bail!("无法从 Releases.md 解析 Deno 版本");
        }
        Ok(v)
    }
    /// Only versions runa can verify (2.1+ publish `<asset>.sha256sum`); older ones stay
    /// resolvable for mise / asdf through `list_all`.
    fn list_remote(&self, net: &Net) -> Result<Vec<RemoteVersion>> {
        Ok(self
            .list_all(net)?
            .into_iter()
            .filter(|v| has_checksums(v))
            .map(|v| RemoteVersion {
                prerelease: crate::version::is_prerelease(&v),
                lts: false,
                version: v,
            })
            .collect())
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let triple = match rust_triple(&t.os, &t.arch) {
            Some(tr) if tr != "i686-pc-windows-msvc" => tr,
            _ => bail!("deno 不支持的平台: {}-{}", t.os, t.arch),
        };
        let file_name = format!("deno-{triple}.zip");
        let base = format!("{REL}v{}/", version.trim_start_matches('v'));
        let url = format!("{base}{file_name}");
        // `<asset>.sha256sum` exists since 2.1 (`deno-<triple>.sha256sum` hashes the
        // unpacked binary, not the archive, so it is not usable here)
        let sha256 = net
            .get_text(&format!("{url}.sha256sum"))
            .ok()
            .filter(|s| s.contains(&file_name))
            .and_then(|s| first_sha256(&s));
        let Some(sha256) = sha256 else {
            bail!(
                "Deno {version} 没有发布安装包校验值（2.1 之前的版本），runa 拒绝安装；请选择 2.1 及以上版本，或通过 mise / asdf 安装"
            );
        };
        Ok(Artifact {
            url,
            file_name,
            sha256: Some(sha256),
        })
    }
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        super::bun::flat_or_bin(install_dir)
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["deno"]
    }
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        Some(vec!["deno"])
    }
}

/// Deno publishes archive checksums since 2.1.0.
pub fn has_checksums(v: &str) -> bool {
    crate::version::compare(v, "2.1.0") != std::cmp::Ordering::Less
}
