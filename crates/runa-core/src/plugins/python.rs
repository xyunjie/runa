use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::{Artifact, Plugin};
use crate::net::{find_in_shasums, Net};
use crate::platform::Target;
use crate::version;

const LATEST_JSON: &str =
    "https://raw.githubusercontent.com/astral-sh/python-build-standalone/latest-release/latest-release.json";
const DOWNLOAD: &str = "https://github.com/astral-sh/python-build-standalone/releases/download/";
const NPMMIRROR_INDEX: &str = "https://registry.npmmirror.com/-/binary/python-build-standalone/";

/// CPython from astral-sh/python-build-standalone (`install_only` builds, no compiling).
///
/// Versions come from the newest release's `SHA256SUMS`, which lists the latest patch of
/// every supported minor series (3.10 – 3.14+), together with checksums.
pub struct Python;

#[derive(Deserialize)]
struct LatestRelease {
    tag: String,
}

#[derive(Deserialize)]
struct MirrorEntry {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

fn triple(t: &Target) -> Result<&'static str> {
    Ok(match (t.os.as_str(), t.arch.as_str()) {
        ("linux", "x64") => "x86_64-unknown-linux-gnu",
        ("linux", "arm64") => "aarch64-unknown-linux-gnu",
        ("macos", "x64") => "x86_64-apple-darwin",
        ("macos", "arm64") => "aarch64-apple-darwin",
        ("windows", "x64") => "x86_64-pc-windows-msvc",
        ("windows", "arm64") => "aarch64-pc-windows-msvc",
        ("windows", "x86") => "i686-pc-windows-msvc",
        (o, a) => bail!("python 不支持的平台: {o}/{a}"),
    })
}

impl Python {
    fn latest_tag(&self, net: &Net) -> Result<String> {
        if net.mirror_mode() == "cn" {
            // npmmirror lists release tags as directories (YYYYMMDD/).
            if let Ok(list) = net.get_json::<Vec<MirrorEntry>>(NPMMIRROR_INDEX) {
                if let Some(tag) = list
                    .iter()
                    .filter(|e| e.kind == "dir")
                    .map(|e| e.name.trim_end_matches('/').to_string())
                    .filter(|n| n.chars().all(|c| c.is_ascii_digit()))
                    .max()
                {
                    return Ok(tag);
                }
            }
        }
        Ok(net.get_json::<LatestRelease>(LATEST_JSON)?.tag)
    }

    /// (tag, SHA256SUMS text)
    fn sums(&self, net: &Net) -> Result<(String, String)> {
        let tag = self.latest_tag(net)?;
        let text = net.get_text(&format!("{DOWNLOAD}{tag}/SHA256SUMS"))?;
        Ok((tag, text))
    }
}

/// Versions available for `triple` in a SHA256SUMS listing.
pub fn versions_in_sums(sums: &str, tag: &str, triple: &str) -> Vec<String> {
    let suffix = format!("+{tag}-{triple}-install_only.tar.gz");
    let mut out: Vec<String> = sums
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter_map(|name| {
            name.strip_prefix("cpython-")?
                .strip_suffix(&suffix)
                .map(|v| v.to_string())
        })
        .filter(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .collect();
    version::sort_desc(&mut out);
    out.dedup();
    out
}

impl Plugin for Python {
    fn name(&self) -> &str {
        "python"
    }
    fn description(&self) -> String {
        "CPython（python-build-standalone 预编译包）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let (tag, sums) = self.sums(net)?;
        Ok(versions_in_sums(&sums, &tag, triple(&Target::current())?))
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let (tag, sums) = self.sums(net)?;
        let file_name = format!("cpython-{version}+{tag}-{}-install_only.tar.gz", triple(t)?);
        let sha256 = find_in_shasums(&sums, &file_name);
        if sha256.is_none() {
            return Err(anyhow!(
                "最新发布 {tag} 中没有 python {version}（python-build-standalone 每个系列只保留最新补丁版本）"
            ));
        }
        Ok(Artifact {
            url: format!("{DOWNLOAD}{tag}/{file_name}"),
            file_name,
            sha256,
        })
    }
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        if cfg!(windows) {
            vec![install_dir.to_path_buf(), install_dir.join("Scripts")]
        } else {
            vec![install_dir.join("bin")]
        }
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["python3", "python"]
    }
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            "python3",
            "python",
            "pip3",
            "pip",
            "pydoc3",
            "python3-config",
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sums() {
        let h = "a".repeat(64);
        let s = format!(
            "{h}  cpython-3.12.15+20261013-x86_64-unknown-linux-gnu-install_only.tar.gz\n\
             {h}  cpython-3.12.15+20261013-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz\n\
             {h}  cpython-3.13.16+20261013-x86_64-unknown-linux-gnu-install_only.tar.gz\n\
             {h}  cpython-3.14.0rc1+20261013-x86_64-unknown-linux-gnu-install_only.tar.gz\n\
             {h}  cpython-3.13.16+20261013-aarch64-apple-darwin-install_only.tar.gz\n"
        );
        let v = versions_in_sums(&s, "20261013", "x86_64-unknown-linux-gnu");
        assert_eq!(v, vec!["3.13.16", "3.12.15"]);
    }
}
