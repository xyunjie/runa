use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::{Artifact, Plugin, RemoteVersion};
use crate::net::{find_in_shasums, Net};
use crate::platform::Target;
use crate::version::{self, VersionReq};

const DIST: &str = "https://nodejs.org/dist/";

/// Node.js from the official builds at nodejs.org/dist (mirror: npmmirror).
pub struct Node;

#[derive(Deserialize)]
struct Release {
    version: String,
    #[serde(default)]
    lts: serde_json::Value,
}

impl Node {
    fn index(&self, net: &Net) -> Result<Vec<Release>> {
        net.get_json(&format!("{DIST}index.json"))
    }
}

fn platform(t: &Target) -> Result<(&'static str, &'static str, &'static str)> {
    let os = match t.os.as_str() {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "win",
        o => bail!("node 不支持的系统: {o}"),
    };
    let arch = match t.arch.as_str() {
        "x64" => "x64",
        "arm64" => "arm64",
        "x86" => "x86",
        a => bail!("node 不支持的架构: {a}"),
    };
    let ext = if os == "win" { "zip" } else { "tar.gz" };
    Ok((os, arch, ext))
}

impl Plugin for Node {
    fn name(&self) -> &str {
        "node"
    }
    fn description(&self) -> String {
        "Node.js JavaScript 运行时（官方预编译包）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let mut v: Vec<String> = self
            .index(net)?
            .into_iter()
            .map(|r| r.version.trim_start_matches('v').to_string())
            .collect();
        version::sort_desc(&mut v);
        Ok(v)
    }
    fn list_remote(&self, net: &Net) -> Result<Vec<RemoteVersion>> {
        let mut v: Vec<RemoteVersion> = self
            .index(net)?
            .into_iter()
            .map(|r| {
                let ver = r.version.trim_start_matches('v').to_string();
                RemoteVersion {
                    prerelease: version::is_prerelease(&ver),
                    lts: r.lts.is_string(),
                    version: ver,
                }
            })
            .collect();
        v.sort_by(|a, b| version::compare(&b.version, &a.version));
        Ok(v)
    }
    fn resolve_remote(&self, net: &Net, req: &VersionReq) -> Result<String> {
        let idx = self.index(net)?;
        let pick = if *req == VersionReq::Lts {
            let lts: Vec<String> = idx
                .iter()
                .filter(|r| r.lts.is_string())
                .map(|r| r.version.trim_start_matches('v').to_string())
                .collect();
            VersionReq::Latest.best(lts.iter())
        } else {
            let all: Vec<String> = idx
                .iter()
                .map(|r| r.version.trim_start_matches('v').to_string())
                .collect();
            req.best(all.iter())
        };
        pick.ok_or_else(|| anyhow!("node 没有匹配的版本"))
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let (os, arch, ext) = platform(t)?;
        let file_name = format!("node-v{version}-{os}-{arch}.{ext}");
        let url = format!("{DIST}v{version}/{file_name}");
        let sums = net
            .get_text(&format!("{DIST}v{version}/SHASUMS256.txt"))
            .ok();
        let sha256 = sums.and_then(|s| find_in_shasums(&s, &file_name));
        Ok(Artifact {
            url,
            file_name,
            sha256,
        })
    }
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        if cfg!(windows) {
            vec![install_dir.to_path_buf()]
        } else {
            vec![install_dir.join("bin")]
        }
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["node"]
    }
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        Some(vec!["node", "npm", "npx", "corepack"])
    }
}
