use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::util::{first_sha256, rust_triple, versions_from_headings};
use super::{Artifact, Plugin};
use crate::net::Net;
use crate::platform::Target;

const DIST: &str = "https://static.rust-lang.org/dist/";
const RELEASES: &str = "https://raw.githubusercontent.com/rust-lang/rust/master/RELEASES.md";

/// Rust toolchains: the official standalone installers from static.rust-lang.org
/// (macOS / Linux; on Windows they are MSI packages, so rustup is used there).
pub struct Rust;

impl Plugin for Rust {
    fn name(&self) -> &str {
        "rust"
    }
    fn description(&self) -> String {
        "Rust 工具链（官方独立安装包；推荐使用 rustup 管理）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let v = versions_from_headings(&net.get_text(RELEASES)?, "Version ");
        if v.is_empty() {
            bail!("无法从 RELEASES.md 解析 Rust 版本");
        }
        Ok(v)
    }
    fn supports_download(&self, t: &Target) -> bool {
        t.os != "windows" && rust_triple(&t.os, &t.arch).is_some()
    }
    fn artifact(&self, net: &Net, version: &str, t: &Target) -> Result<Artifact> {
        if t.os == "windows" {
            bail!("Windows 上的 Rust 官方独立安装包是 MSI，runa 不直接下载；请通过 rustup 安装");
        }
        let Some(triple) = rust_triple(&t.os, &t.arch) else {
            bail!("rust 不支持的平台: {}-{}", t.os, t.arch);
        };
        let file_name = format!("rust-{version}-{triple}.tar.gz");
        let url = format!("{DIST}{file_name}");
        let sha256 = net
            .get_text(&format!("{url}.sha256"))
            .ok()
            .and_then(|s| first_sha256(&s));
        Ok(Artifact {
            url,
            file_name,
            sha256,
        })
    }
    /// The standalone tarball is an installer: run its `install.sh` into a prefix
    /// inside the staging dir (docs left out), then keep only that prefix.
    fn post_extract(&self, staged: &Path) -> Result<()> {
        let script = staged.join("install.sh");
        if !script.is_file() {
            return Ok(());
        }
        let comps: Vec<String> = std::fs::read_to_string(staged.join("components"))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|c| !c.is_empty() && !c.starts_with("rust-docs"))
            .map(str::to_string)
            .collect();
        let prefix = staged.join(".runa-prefix");
        let mut cmd = Command::new("sh");
        cmd.arg(&script)
            .arg(format!("--prefix={}", prefix.display()))
            .arg("--disable-ldconfig");
        if !comps.is_empty() {
            cmd.arg(format!("--components={}", comps.join(",")));
        }
        let out = cmd.output().context("无法运行 Rust 的 install.sh")?;
        if !out.status.success() {
            bail!(
                "Rust install.sh 失败:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        for e in std::fs::read_dir(staged)?.flatten() {
            if e.file_name() != ".runa-prefix" {
                let p = e.path();
                if e.file_type()?.is_dir() {
                    std::fs::remove_dir_all(&p)?;
                } else {
                    std::fs::remove_file(&p)?;
                }
            }
        }
        for e in std::fs::read_dir(&prefix)?.flatten() {
            std::fs::rename(e.path(), staged.join(e.file_name()))?;
        }
        std::fs::remove_dir(&prefix)?;
        Ok(())
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["rustc"]
    }
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            "rustc",
            "cargo",
            "rustdoc",
            "rustfmt",
            "cargo-fmt",
            "cargo-clippy",
            "clippy-driver",
            "rust-gdb",
            "rust-lldb",
            "rust-analyzer",
        ])
    }
}
