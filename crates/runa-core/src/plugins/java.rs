use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::{Artifact, Plugin, RemoteVersion};
use crate::net::Net;
use crate::platform::Target;
use crate::version::{self, VersionReq};

const API: &str = "https://api.adoptium.net/v3";

/// Eclipse Temurin JDKs via the Adoptium API.
pub struct Java;

#[derive(Deserialize)]
struct Available {
    available_releases: Vec<u32>,
    #[serde(default)]
    available_lts_releases: Vec<u32>,
    most_recent_lts: u32,
}

#[derive(Deserialize)]
struct VersionData {
    major: u32,
    #[serde(default)]
    minor: u32,
    #[serde(default)]
    security: u32,
    #[serde(default)]
    patch: Option<u32>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    link: String,
    #[serde(default)]
    checksum: Option<String>,
}

#[derive(Deserialize)]
struct Binary {
    package: Package,
}

/// Element of /v3/assets/latest/...
#[derive(Deserialize)]
struct LatestAsset {
    binary: Binary,
    version: VersionData,
}

/// Element of /v3/assets/feature_releases/...
#[derive(Deserialize)]
struct FeatureRelease {
    binaries: Vec<Binary>,
    version_data: VersionData,
}

/// `21.0.12` or `21.0.12.1` (Adoptium re-spins add a 4th component).
fn format_version(v: &VersionData) -> String {
    match v.patch {
        Some(p) if p > 0 => format!("{}.{}.{}.{}", v.major, v.minor, v.security, p),
        _ => format!("{}.{}.{}", v.major, v.minor, v.security),
    }
}

fn platform(t: &Target) -> Result<(&'static str, &'static str)> {
    let os = match t.os.as_str() {
        "linux" => "linux",
        "macos" => "mac",
        "windows" => "windows",
        o => bail!("java 不支持的系统: {o}"),
    };
    let arch = match t.arch.as_str() {
        "x64" => "x64",
        "arm64" => "aarch64",
        "x86" => "x86",
        a => bail!("java 不支持的架构: {a}"),
    };
    Ok((os, arch))
}

impl Java {
    fn available(&self, net: &Net) -> Result<Available> {
        net.get_json(&format!("{API}/info/available_releases"))
    }

    fn latest(&self, net: &Net, major: u32, t: &Target) -> Result<(String, Artifact)> {
        let (os, arch) = platform(t)?;
        let url = format!(
            "{API}/assets/latest/{major}/hotspot?architecture={arch}&image_type=jdk&os={os}&vendor=eclipse"
        );
        let list: Vec<LatestAsset> = net.get_json(&url)?;
        let a = list
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Adoptium 没有 Java {major} 的 {os}/{arch} 构建"))?;
        let p = a.binary.package;
        Ok((
            format_version(&a.version),
            Artifact {
                url: p.link,
                file_name: p.name,
                sha256: p.checksum,
            },
        ))
    }

    fn feature_releases(
        &self,
        net: &Net,
        major: u32,
        t: &Target,
    ) -> Result<Vec<(String, Artifact)>> {
        let (os, arch) = platform(t)?;
        let url = format!(
            "{API}/assets/feature_releases/{major}/ga?architecture={arch}&image_type=jdk&os={os}&vendor=eclipse&jvm_impl=hotspot&page_size=50&sort_order=DESC"
        );
        let list: Vec<FeatureRelease> = net.get_json(&url)?;
        Ok(list
            .into_iter()
            .filter_map(|r| {
                let v = format_version(&r.version_data);
                let b = r.binaries.into_iter().next()?;
                Some((
                    v,
                    Artifact {
                        url: b.package.link,
                        file_name: b.package.name,
                        sha256: b.package.checksum,
                    },
                ))
            })
            .collect())
    }
}

impl Plugin for Java {
    fn name(&self) -> &str {
        "java"
    }
    fn description(&self) -> String {
        "Java JDK（Eclipse Temurin / Adoptium）".into()
    }
    /// Feature releases (majors); `runa install java 21` picks the newest 21.x.
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let a = self.available(net)?;
        let mut v: Vec<String> = a.available_releases.iter().map(|m| m.to_string()).collect();
        version::sort_desc(&mut v);
        Ok(v)
    }
    fn list_remote(&self, net: &Net) -> Result<Vec<RemoteVersion>> {
        let a = self.available(net)?;
        let mut majors = a.available_releases.clone();
        majors.sort_unstable_by(|x, y| y.cmp(x));
        Ok(majors
            .into_iter()
            .map(|m| RemoteVersion {
                version: m.to_string(),
                lts: a.available_lts_releases.contains(&m),
                prerelease: false,
            })
            .collect())
    }
    fn resolve_remote(&self, net: &Net, req: &VersionReq) -> Result<String> {
        let t = Target::current();
        let major = match req {
            VersionReq::Latest | VersionReq::Lts => self.available(net)?.most_recent_lts as u64,
            VersionReq::Prefix(c) | VersionReq::AtLeast(c) if c.len() == 1 => c[0],
            VersionReq::Prefix(c) | VersionReq::AtLeast(c) => {
                // 1.8 -> 8
                if c[0] == 1 && c.len() > 1 {
                    c[1]
                } else {
                    c[0]
                }
            }
            VersionReq::Exact(s) => *version::components(s)
                .first()
                .ok_or_else(|| anyhow!("无效的 Java 版本: {s}"))?,
            VersionReq::System => bail!("system 不能安装"),
        } as u32;
        let (latest, _) = self.latest(net, major, &t)?;
        if req.matches(&latest) || matches!(req, VersionReq::Latest | VersionReq::Lts) {
            return Ok(latest);
        }
        let all: Vec<String> = self
            .feature_releases(net, major, &t)?
            .into_iter()
            .map(|(v, _)| v)
            .collect();
        req.best(all.iter())
            .ok_or_else(|| anyhow!("java 没有匹配 {req:?} 的版本"))
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let major = *version::components(version)
            .first()
            .ok_or_else(|| anyhow!("无效的 Java 版本"))? as u32;
        let (latest, art) = self.latest(net, major, t)?;
        if latest == version {
            return Ok(art);
        }
        self.feature_releases(net, major, t)?
            .into_iter()
            .find(|(v, _)| v == version)
            .map(|(_, a)| a)
            .ok_or_else(|| anyhow!("Adoptium 上找不到 Java {version}"))
    }
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        vec![java_home(install_dir).join("bin")]
    }
    fn env_vars(&self, install_dir: &Path) -> Vec<(String, String)> {
        vec![(
            "JAVA_HOME".into(),
            java_home(install_dir).display().to_string(),
        )]
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["java"]
    }
    fn version_args(&self) -> Vec<&'static str> {
        vec!["-version"]
    }
    fn parse_version_output(&self, out: &str) -> Option<String> {
        // `openjdk version "21.0.2"` / `java version "1.8.0_392"`
        let quoted = out.split('"').nth(1).unwrap_or(out);
        if let Some(rest) = quoted.strip_prefix("1.8.0_") {
            let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            return Some(format!("8.0.{n}"));
        }
        let v = super::first_version(quoted).or_else(|| {
            let d: String = quoted.chars().take_while(|c| c.is_ascii_digit()).collect();
            (!d.is_empty()).then_some(d)
        })?;
        Some(v)
    }
}

/// macOS JDK archives contain `Contents/Home`.
pub fn java_home(install_dir: &Path) -> PathBuf {
    let mac = install_dir.join("Contents").join("Home");
    if mac.join("bin").is_dir() {
        mac
    } else {
        install_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_java_versions() {
        let j = Java;
        assert_eq!(
            j.parse_version_output("openjdk version \"21.0.2\" 2024-01-16 LTS")
                .as_deref(),
            Some("21.0.2")
        );
        assert_eq!(
            j.parse_version_output("java version \"1.8.0_392\"")
                .as_deref(),
            Some("8.0.392")
        );
        assert_eq!(
            j.parse_version_output("openjdk version \"22\" 2024-03-19")
                .as_deref(),
            Some("22")
        );
    }

    #[test]
    fn version_format() {
        let v = VersionData {
            major: 21,
            minor: 0,
            security: 12,
            patch: Some(1),
        };
        assert_eq!(format_version(&v), "21.0.12.1");
        let v = VersionData {
            major: 17,
            minor: 0,
            security: 9,
            patch: None,
        };
        assert_eq!(format_version(&v), "17.0.9");
    }
}
