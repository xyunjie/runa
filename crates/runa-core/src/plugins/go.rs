use std::path::Path;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::{Artifact, Plugin};
use crate::net::Net;
use crate::platform::Target;
use crate::version::{self, VersionReq};

const DL: &str = "https://go.dev/dl/";

/// Go toolchains from go.dev/dl (mirror: golang.google.cn).
pub struct Go;

#[derive(Deserialize)]
struct Release {
    version: String,
    stable: bool,
    #[serde(default)]
    files: Vec<File>,
}

#[derive(Deserialize)]
struct File {
    filename: String,
    sha256: String,
}

impl Go {
    fn releases(&self, net: &Net) -> Result<Vec<Release>> {
        net.get_json(&format!("{DL}?mode=json&include=all"))
    }
}

fn file_name(version: &str, t: &Target) -> Result<String> {
    let os = match t.os.as_str() {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        o => bail!("go 不支持的系统: {o}"),
    };
    let arch = match t.arch.as_str() {
        "x64" => "amd64",
        "arm64" => "arm64",
        "x86" => "386",
        a => bail!("go 不支持的架构: {a}"),
    };
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    Ok(format!("go{version}.{os}-{arch}.{ext}"))
}

impl Plugin for Go {
    fn name(&self) -> &str {
        "go"
    }
    fn description(&self) -> String {
        "Go 工具链（go.dev 官方发行版）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let mut v: Vec<String> = self
            .releases(net)?
            .into_iter()
            .map(|r| r.version.trim_start_matches("go").to_string())
            .collect();
        version::sort_desc(&mut v);
        v.dedup();
        Ok(v)
    }
    fn resolve_remote(&self, net: &Net, req: &VersionReq) -> Result<String> {
        let rels = self.releases(net)?;
        let all: Vec<String> = rels
            .iter()
            .filter(|r| r.stable || !req.is_fuzzy())
            .map(|r| r.version.trim_start_matches("go").to_string())
            .collect();
        let req = if *req == VersionReq::Lts {
            VersionReq::Latest
        } else {
            req.clone()
        };
        req.best(all.iter())
            .ok_or_else(|| anyhow!("go 没有匹配的版本"))
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let name = file_name(version, t)?;
        let sha256 = self.releases(net).ok().and_then(|rels| {
            rels.into_iter()
                .flat_map(|r| r.files)
                .find(|f| f.filename == name)
                .map(|f| f.sha256)
        });
        Ok(Artifact {
            url: format!("{DL}{name}"),
            file_name: name,
            sha256,
        })
    }
    fn env_vars(&self, install_dir: &Path) -> Vec<(String, String)> {
        vec![("GOROOT".into(), install_dir.display().to_string())]
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["go"]
    }
    fn version_args(&self) -> Vec<&'static str> {
        vec!["version"]
    }
}
