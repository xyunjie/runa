use anyhow::{bail, Result};

use super::util::versions_from_headings;
use super::{Artifact, Plugin};
use crate::net::Net;
use crate::platform::Target;

const RELEASES: &str = "https://raw.githubusercontent.com/rust-lang/rust/master/RELEASES.md";

/// Rust toolchains. runa lists versions but installs only through rustup / mise / asdf.
pub struct Rust;

impl Plugin for Rust {
    fn name(&self) -> &str {
        "rust"
    }
    fn description(&self) -> String {
        "Rust 工具链（通过 rustup / mise / asdf 安装）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        let v = versions_from_headings(&net.get_text(RELEASES)?, "Version ");
        if v.is_empty() {
            bail!("无法从 RELEASES.md 解析 Rust 版本");
        }
        Ok(v)
    }
    fn supports_download(&self, _t: &Target) -> bool {
        false
    }
    fn artifact(&self, _net: &Net, _version: &str, _t: &Target) -> Result<Artifact> {
        bail!("runa 不直接下载 Rust；请通过 rustup（推荐）、mise 或 asdf 安装")
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
