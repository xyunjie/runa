//! Declarative JSON plugins for extra tools.
//!
//! Put files in `~/.runa/plugins/*.json`. Each file holds one manifest or an array of them:
//!
//! ```json
//! {
//!   "name": "deno",
//!   "description": "Deno 运行时",
//!   "source": { "type": "github-releases", "repo": "denoland/deno", "tagPrefix": "v" },
//!   "url": "https://github.com/denoland/deno/releases/download/v{version}/{asset}",
//!   "assets": { "linux/x64": "deno-x86_64-unknown-linux-gnu.zip" },
//!   "sha256Url": "{url}.sha256sum",
//!   "bin": [""],
//!   "env": { "DENO_DIR": "{install_dir}/cache" }
//! }
//! ```
//!
//! `source.type` may also be `static` with an explicit `versions` list.
//! Placeholders: `{version}`, `{asset}`, `{os}`, `{arch}`, `{url}` (sha256Url only), `{install_dir}` (env).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use super::{Artifact, Plugin};
use crate::net::{find_in_shasums, Net};
use crate::platform::Target;
use crate::version;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SourceDef {
    GithubReleases {
        repo: String,
        #[serde(default, rename = "tagPrefix", alias = "stripPrefix")]
        tag_prefix: String,
    },
    Static {
        versions: Vec<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BinDef {
    One(String),
    Many(Vec<String>),
}

impl Default for BinDef {
    fn default() -> Self {
        BinDef::One("bin".into())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: SourceDef,
    pub url: String,
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
    #[serde(default, rename = "sha256Url")]
    pub sha256_url: Option<String>,
    #[serde(default)]
    pub bin: BinDef,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, rename = "detect")]
    pub detect: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(Box<Manifest>),
    Many(Vec<Manifest>),
}

pub fn parse(text: &str) -> Result<Vec<Manifest>> {
    let v: OneOrMany = serde_json::from_str(text)?;
    let list = match v {
        OneOrMany::One(m) => vec![*m],
        OneOrMany::Many(v) => v,
    };
    for m in &list {
        validate(m)?;
    }
    Ok(list)
}

fn validate(m: &Manifest) -> Result<()> {
    let ok = !m.name.is_empty()
        && m.name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !ok {
        bail!("插件名无效: {:?}（仅允许小写字母、数字、- 和 _）", m.name);
    }
    if !m.url.starts_with("https://") {
        bail!("插件 {} 的 url 必须使用 https://", m.name);
    }
    for k in m.env.keys() {
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            bail!("插件 {} 的环境变量名无效: {k}", m.name);
        }
    }
    let bins = match &m.bin {
        BinDef::One(s) => vec![s.clone()],
        BinDef::Many(v) => v.clone(),
    };
    for b in bins {
        crate::archive::safe_relative(Path::new(&b))
            .with_context(|| format!("插件 {} 的 bin 路径无效", m.name))?;
    }
    Ok(())
}

/// Load every manifest in a directory. Returns (manifest, warning) pairs.
pub fn load_dir(dir: &Path) -> Vec<(Option<Manifest>, Option<String>)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    for p in files {
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&p)
            .map_err(anyhow::Error::from)
            .and_then(|t| parse(&t))
        {
            Ok(list) => out.extend(list.into_iter().map(|m| (Some(m), None))),
            Err(e) => out.push((None, Some(format!("加载插件 {} 失败: {e:#}", p.display())))),
        }
    }
    out
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut s = template.to_string();
    for (k, v) in pairs {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

impl Plugin for Manifest {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> String {
        if self.description.is_empty() {
            format!("{}（JSON 插件）", self.name)
        } else {
            self.description.clone()
        }
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let mut v = match &self.source {
            SourceDef::Static { versions } => versions.clone(),
            SourceDef::GithubReleases { repo, tag_prefix } => {
                let url = format!("https://api.github.com/repos/{repo}/releases?per_page=100");
                let rels: Vec<GhRelease> = net.get_json(&url)?;
                rels.into_iter()
                    .filter(|r| !r.prerelease && !r.draft)
                    .map(|r| {
                        r.tag_name
                            .strip_prefix(tag_prefix.as_str())
                            .unwrap_or(&r.tag_name)
                            .to_string()
                    })
                    .collect()
            }
        };
        version::sort_desc(&mut v);
        Ok(v)
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        let asset_tpl = self
            .assets
            .get(&t.key())
            .ok_or_else(|| anyhow!("插件 {} 不支持平台 {}", self.name, t.key()))?;
        let base = [
            ("version", version),
            ("os", t.os.as_str()),
            ("arch", t.arch.as_str()),
        ];
        let asset = fill(asset_tpl, &base);
        let mut pairs = base.to_vec();
        pairs.push(("asset", asset.as_str()));
        let url = fill(&self.url, &pairs);
        let file_name = url.rsplit('/').next().unwrap_or(&asset).to_string();
        let sha256 = match &self.sha256_url {
            None => None,
            Some(tpl) => {
                let mut p = pairs.clone();
                p.push(("url", url.as_str()));
                let text = net.get_text(&fill(tpl, &p))?;
                let bare = text
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                find_in_shasums(&text, &file_name).or_else(|| (bare.len() == 64).then_some(bare))
            }
        };
        Ok(Artifact {
            url,
            file_name,
            sha256,
        })
    }
    fn bin_dirs(&self, install_dir: &Path) -> Vec<PathBuf> {
        let list = match &self.bin {
            BinDef::One(s) => vec![s.clone()],
            BinDef::Many(v) => v.clone(),
        };
        list.iter()
            .map(|b| {
                if b.is_empty() || b == "." {
                    install_dir.to_path_buf()
                } else {
                    install_dir.join(b)
                }
            })
            .collect()
    }
    fn env_vars(&self, install_dir: &Path) -> Vec<(String, String)> {
        let d = install_dir.display().to_string();
        self.env
            .iter()
            .map(|(k, v)| (k.clone(), v.replace("{install_dir}", &d)))
            .collect()
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec![]
    }
    fn builtin(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../../../examples/plugins/sample.json");

    #[test]
    fn sample_manifests_parse() {
        let list = parse(SAMPLE).unwrap();
        assert!(list.iter().any(|m| m.name == "deno"));
        let deno = list.iter().find(|m| m.name == "deno").unwrap();
        assert_eq!(deno.bin_dirs(Path::new("/x")), vec![PathBuf::from("/x")]);
    }

    #[test]
    fn rejects_bad_manifests() {
        let bad_name = r#"{"name":"Bad Name","source":{"type":"static","versions":["1"]},"url":"https://x/{version}"}"#;
        assert!(parse(bad_name).is_err());
        let http = r#"{"name":"x","source":{"type":"static","versions":["1"]},"url":"http://x/{version}"}"#;
        assert!(parse(http).is_err());
        let slip = r#"{"name":"x","source":{"type":"static","versions":["1"]},"url":"https://x/","bin":"../../etc"}"#;
        assert!(parse(slip).is_err());
    }

    #[test]
    fn placeholders() {
        let m = &parse(
            r#"{"name":"t","source":{"type":"static","versions":["1.0.0","1.2.0"]},
                "url":"https://dl/{version}/{asset}","assets":{"linux/x64":"t-{version}-{os}-{arch}.tar.gz"},
                "env":{"T_HOME":"{install_dir}"}}"#,
        )
        .unwrap()[0];
        let net = Net::new(&crate::settings::Settings::default()).unwrap();
        let a = m
            .artifact(&net, "1.2.0", &Target::new("linux", "x64"))
            .unwrap();
        assert_eq!(a.url, "https://dl/1.2.0/t-1.2.0-linux-x64.tar.gz");
        assert_eq!(a.file_name, "t-1.2.0-linux-x64.tar.gz");
        assert!(m
            .artifact(&net, "1.2.0", &Target::new("windows", "x64"))
            .is_err());
        assert_eq!(m.list_all(&net).unwrap(), vec!["1.2.0", "1.0.0"]);
        assert_eq!(
            m.env_vars(Path::new("/i")),
            vec![("T_HOME".to_string(), "/i".to_string())]
        );
    }
}
