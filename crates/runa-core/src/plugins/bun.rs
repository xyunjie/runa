use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::Deserialize;

use super::{Artifact, Plugin};
use crate::net::{find_in_shasums, Net};
use crate::platform::Target;
use crate::version;

const REL: &str = "https://github.com/oven-sh/bun/releases/download/";

/// Bun from its GitHub releases (checksums from the release's SHASUMS256.txt). The
/// version list comes from the npm registry, which is not rate limited like the
/// GitHub API.
pub struct Bun;

#[derive(Deserialize)]
struct Pkg {
    versions: BTreeMap<String, serde_json::Value>,
}

pub fn asset(t: &Target) -> Result<&'static str> {
    Ok(match (t.os.as_str(), t.arch.as_str()) {
        ("macos", "arm64") => "bun-darwin-aarch64",
        ("macos", "x64") => "bun-darwin-x64",
        ("linux", "arm64") => "bun-linux-aarch64",
        ("linux", "x64") => "bun-linux-x64",
        // no native Windows arm64 build: the x64 one runs under emulation
        ("windows", "x64" | "arm64") => "bun-windows-x64",
        (o, a) => bail!("bun 不支持的平台: {o}-{a}"),
    })
}

/// Runa's own installs have `bun` at the top; adopted ones (`~/.bun`, kegs) use `bin/`.
pub fn flat_or_bin(install_dir: &Path) -> Vec<PathBuf> {
    let bin = install_dir.join("bin");
    if bin.is_dir() {
        vec![bin]
    } else {
        vec![install_dir.to_path_buf()]
    }
}

impl Plugin for Bun {
    fn name(&self) -> &str {
        "bun"
    }
    fn description(&self) -> String {
        "Bun JavaScript 运行时（GitHub 官方发行版）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let p: Pkg = net.get_json("https://registry.npmjs.org/bun")?;
        let mut v: Vec<String> = p
            .versions
            .into_keys()
            .filter(|v| !v.contains("canary"))
            .collect();
        version::sort_desc(&mut v);
        Ok(v)
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let file_name = format!("{}.zip", asset(t)?);
        let base = format!("{REL}bun-v{version}/");
        let sha256 = net
            .get_text(&format!("{base}SHASUMS256.txt"))
            .ok()
            .and_then(|s| find_in_shasums(&s, &file_name));
        Ok(Artifact {
            url: format!("{base}{file_name}"),
            file_name,
            sha256,
        })
    }
    /// `bunx` is bun itself under another name (the official installer does the same).
    fn post_extract(&self, staged: &Path) -> Result<()> {
        if cfg!(windows) {
            let exe = staged.join("bun.exe");
            if exe.is_file() {
                std::fs::copy(&exe, staged.join("bunx.exe"))?;
            }
        } else {
            #[cfg(unix)]
            if staged.join("bun").is_file() {
                let _ = std::os::unix::fs::symlink("bun", staged.join("bunx"));
            }
        }
        Ok(())
    }
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        flat_or_bin(install_dir)
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["bun"]
    }
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        Some(vec!["bun", "bunx"])
    }
}
