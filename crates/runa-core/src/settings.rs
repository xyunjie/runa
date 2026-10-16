//! Persistent settings: download mirror and network proxy.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirrorRule {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// `off` or `cn` (built-in China mirror preset).
    pub mirror: String,
    /// Extra user-defined URL prefix rewrites, applied before the preset.
    pub mirror_rules: Vec<MirrorRule>,
    /// `system` (env vars / OS settings), `off`, or a proxy URL
    /// (`http://`, `https://`, `socks5://`, `socks5h://`).
    pub proxy: String,
    /// Preferred installer per tool for new versions: `auto` (default: a detected
    /// version manager, else runa's download), `runa`, or a manager name (`nvm`,
    /// `nvm-windows`, `fnm`, `pyenv`, `pyenv-win`, `sdkman`, `asdf`, `mise`).
    pub install_via: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mirror: "off".into(),
            mirror_rules: vec![],
            proxy: "system".into(),
            install_via: Default::default(),
        }
    }
}

/// Built-in mirror presets.
pub fn preset_rules(mode: &str) -> Vec<MirrorRule> {
    let r = |f: &str, t: &str| MirrorRule {
        from: f.into(),
        to: t.into(),
    };
    match mode {
        "cn" => vec![
            r(
                "https://nodejs.org/dist/",
                "https://npmmirror.com/mirrors/node/",
            ),
            r("https://go.dev/dl/", "https://golang.google.cn/dl/"),
            r(
                "https://github.com/astral-sh/python-build-standalone/releases/download/",
                "https://registry.npmmirror.com/-/binary/python-build-standalone/",
            ),
        ],
        _ => vec![],
    }
}

pub const MIRROR_MODES: &[&str] = &["off", "cn"];

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        let mut s: Settings = match std::fs::read_to_string(path) {
            Ok(t) => {
                serde_json::from_str(&t).with_context(|| format!("解析 {} 失败", path.display()))?
            }
            Err(_) => Settings::default(),
        };
        // Environment overrides (handy for CI): RUNA_MIRROR / RUNA_PROXY.
        if let Ok(m) = std::env::var("RUNA_MIRROR") {
            if !m.trim().is_empty() {
                s.mirror = m.trim().to_ascii_lowercase();
            }
        }
        if let Ok(p) = std::env::var("RUNA_PROXY") {
            if !p.trim().is_empty() {
                s.proxy = p.trim().to_string();
            }
        }
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)? + "\n")?;
        Ok(())
    }

    pub fn set_mirror(&mut self, mode: &str) -> Result<()> {
        let m = mode.trim().to_ascii_lowercase();
        if !MIRROR_MODES.contains(&m.as_str()) {
            bail!("未知镜像: {mode}（可选: {}）", MIRROR_MODES.join(" / "));
        }
        self.mirror = m;
        Ok(())
    }

    /// Set the preferred installer for `tool` (`auto` removes the preference).
    pub fn set_install_via(&mut self, tool: &str, via: &str) -> Result<()> {
        let v = via.trim().to_ascii_lowercase();
        if !crate::managers::VIAS.contains(&v.as_str()) {
            bail!(
                "未知安装方式: {via}（可选: {}）",
                crate::managers::VIAS.join(" / ")
            );
        }
        if v == "auto" {
            self.install_via.remove(tool);
        } else {
            self.install_via.insert(tool.to_string(), v);
        }
        Ok(())
    }

    pub fn install_via(&self, tool: &str) -> &str {
        self.install_via
            .get(tool)
            .map(String::as_str)
            .unwrap_or("auto")
    }

    pub fn set_proxy(&mut self, value: &str) -> Result<()> {
        let v = value.trim();
        let lower = v.to_ascii_lowercase();
        if lower == "off" || lower == "system" {
            self.proxy = lower;
            return Ok(());
        }
        validate_proxy_url(v)?;
        self.proxy = v.to_string();
        Ok(())
    }

    /// All active rewrite rules: user rules first, then the preset.
    pub fn rules(&self) -> Vec<MirrorRule> {
        let mut r = self.mirror_rules.clone();
        r.extend(preset_rules(&self.mirror));
        r
    }

    pub fn rewrite_url(&self, url: &str) -> String {
        for rule in self.rules() {
            if let Some(rest) = url.strip_prefix(&rule.from) {
                return format!("{}{}", rule.to, rest);
            }
        }
        url.to_string()
    }
}

pub fn validate_proxy_url(v: &str) -> Result<()> {
    let ok_scheme = ["http://", "https://", "socks5://", "socks5h://"]
        .iter()
        .any(|s| v.starts_with(s));
    if !ok_scheme {
        bail!("代理地址需以 http:// https:// socks5:// 或 socks5h:// 开头，或使用 off / system");
    }
    let rest = v.split("://").nth(1).unwrap_or("");
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        bail!("代理地址无效: {v}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("settings.json");
        let s = Settings::load(&p).unwrap();
        assert_eq!(s, Settings::default());
        let mut s2 = s.clone();
        s2.set_mirror("CN").unwrap();
        s2.set_proxy("socks5://127.0.0.1:1080").unwrap();
        s2.save(&p).unwrap();
        let s3: Settings = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(s3.mirror, "cn");
        assert_eq!(s3.proxy, "socks5://127.0.0.1:1080");
        // unknown fields / partial files are tolerated
        let partial: Settings = serde_json::from_str(r#"{"mirror":"cn"}"#).unwrap();
        assert_eq!(partial.proxy, "system");
    }

    #[test]
    fn validation() {
        let mut s = Settings::default();
        assert!(s.set_mirror("nope").is_err());
        assert!(s.set_proxy("ftp://x").is_err());
        assert!(s.set_proxy("http://").is_err());
        assert!(s.set_proxy("OFF").is_ok());
        assert_eq!(s.proxy, "off");
        assert!(s.set_proxy("http://user:pw@proxy.local:8080").is_ok());
    }

    #[test]
    fn mirror_rewrite() {
        let mut s = Settings::default();
        let u = "https://nodejs.org/dist/v20.0.0/node-v20.0.0-linux-x64.tar.gz";
        assert_eq!(s.rewrite_url(u), u);
        s.mirror = "cn".into();
        assert_eq!(
            s.rewrite_url(u),
            "https://npmmirror.com/mirrors/node/v20.0.0/node-v20.0.0-linux-x64.tar.gz"
        );
        assert_eq!(
            s.rewrite_url("https://go.dev/dl/?mode=json"),
            "https://golang.google.cn/dl/?mode=json"
        );
        s.mirror_rules.push(MirrorRule {
            from: "https://nodejs.org/dist/".into(),
            to: "https://my/".into(),
        });
        assert!(s.rewrite_url(u).starts_with("https://my/"));
    }
}
