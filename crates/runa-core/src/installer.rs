//! Download -> verify -> extract -> atomically move into `installs/<tool>/<version>`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::archive;
use crate::net::{sha256_file, Net};
use crate::paths::Paths;
use crate::platform::Target;
use crate::plugins::Plugin;

/// Progress events, shared by CLI progress bars and GUI events.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "lowercase")]
pub enum Progress {
    Resolving {
        tool: String,
        spec: String,
    },
    Downloading {
        tool: String,
        version: String,
        done: u64,
        total: Option<u64>,
    },
    Verifying {
        tool: String,
        version: String,
        checksum: bool,
    },
    Extracting {
        tool: String,
        version: String,
    },
    Done {
        tool: String,
        version: String,
    },
    Warning {
        message: String,
    },
}

/// Metadata stored next to every install as `installs/<tool>/<version>.receipt.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Receipt {
    pub tool: String,
    pub version: String,
    /// `download` or `adopted`.
    pub source: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub verified: bool,
    pub installed_at: u64,
    /// For adopted runtimes: the original location (never modified by runa).
    pub origin: Option<PathBuf>,
    /// Overrides the install dir as runtime root (adoption fallback on Windows).
    pub home: Option<PathBuf>,
    /// Overrides plugin bin dirs (absolute paths).
    pub bin_dirs: Vec<PathBuf>,
}

pub fn receipt_path(paths: &Paths, tool: &str, version: &str) -> PathBuf {
    paths.tool_dir(tool).join(format!("{version}.receipt.json"))
}

pub fn read_receipt(paths: &Paths, tool: &str, version: &str) -> Option<Receipt> {
    let t = std::fs::read_to_string(receipt_path(paths, tool, version)).ok()?;
    serde_json::from_str(&t).ok()
}

pub fn write_receipt(paths: &Paths, r: &Receipt) -> Result<()> {
    let p = receipt_path(paths, &r.tool, &r.version);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(p, serde_json::to_string_pretty(r)?)?;
    Ok(())
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Version strings become directory names: refuse anything path-like.
pub fn validate_version_name(v: &str) -> Result<()> {
    let bad = v.is_empty()
        || v.starts_with('.')
        || v.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || v.contains("..");
    if bad {
        anyhow::bail!("非法版本号: {v:?}");
    }
    Ok(())
}

/// Install a concrete `version` of `plugin`. No-op if already installed.
pub fn install(
    paths: &Paths,
    net: &Net,
    plugin: &dyn Plugin,
    version: &str,
    require_checksum: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf> {
    validate_version_name(version)?;
    let tool = plugin.name().to_string();
    let dest = paths.install_dir(&tool, version);
    if dest.is_dir() {
        progress(Progress::Done {
            tool,
            version: version.into(),
        });
        return Ok(dest);
    }
    paths.ensure()?;
    let target = Target::current();
    let art = plugin.artifact(net, version, &target)?;
    if art.sha256.is_none() && (require_checksum || require_checksum_env()) {
        anyhow::bail!("上游未提供 {tool} {version} 的 SHA256 校验值；--require-checksum 拒绝安装");
    }
    let file_name = sanitize_file_name(&art.file_name);
    let archive_path = paths.downloads().join(&file_name);

    let reuse = match (&art.sha256, archive_path.is_file()) {
        (Some(h), true) => sha256_file(&archive_path)
            .map(|a| a.eq_ignore_ascii_case(h))
            .unwrap_or(false),
        _ => false,
    };
    if !reuse {
        if art.sha256.is_none() {
            progress(Progress::Warning {
                message: format!("上游未提供 {tool} {version} 的 SHA256 校验值，将跳过完整性校验"),
            });
        }
        let (t2, v2) = (tool.clone(), version.to_string());
        net.download(
            &art.url,
            &archive_path,
            art.sha256.as_deref(),
            &mut |done, total| {
                progress(Progress::Downloading {
                    tool: t2.clone(),
                    version: v2.clone(),
                    done,
                    total,
                })
            },
        )?;
    }
    progress(Progress::Verifying {
        tool: tool.clone(),
        version: version.into(),
        checksum: art.sha256.is_some(),
    });

    progress(Progress::Extracting {
        tool: tool.clone(),
        version: version.into(),
    });
    let tmp = paths
        .tool_dir(&tool)
        .join(format!(".tmp-{version}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let res = (|| -> Result<()> {
        archive::extract(&archive_path, &tmp)?;
        archive::flatten_single_root(&tmp)?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("移动到 {} 失败", dest.display()))?;
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    write_receipt(
        paths,
        &Receipt {
            tool: tool.clone(),
            version: version.into(),
            source: "download".into(),
            url: Some(net.rewrite(&art.url)),
            sha256: art.sha256.clone(),
            verified: art.sha256.is_some(),
            installed_at: now(),
            ..Default::default()
        },
    )?;
    let _ = std::fs::remove_file(&archive_path);
    progress(Progress::Done {
        tool,
        version: version.into(),
    });
    Ok(dest)
}

/// `RUNA_REQUIRE_CHECKSUM=1` makes every install demand a publisher checksum.
pub fn require_checksum_env() -> bool {
    std::env::var("RUNA_REQUIRE_CHECKSUM")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

fn sanitize_file_name(name: &str) -> String {
    let n = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    n.replace(['%', '?', '&', '='], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_names() {
        assert!(validate_version_name("20.11.1").is_ok());
        assert!(validate_version_name("21.0.2+13").is_ok());
        assert!(validate_version_name("../x").is_err());
        assert!(validate_version_name(".hidden").is_err());
        assert!(validate_version_name("a/b").is_err());
        assert!(validate_version_name("").is_err());
    }
}
