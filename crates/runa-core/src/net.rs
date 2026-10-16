//! HTTP access with mirror rewriting, proxy support, progress and SHA-256 verification.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::settings::Settings;

pub struct Net {
    client: reqwest::blocking::Client,
    settings: Settings,
}

impl Net {
    pub fn new(settings: &Settings) -> Result<Self> {
        let mut b = reqwest::blocking::Client::builder()
            .user_agent(concat!("runa/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(20))
            .timeout(None::<Duration>);
        match settings.proxy.as_str() {
            "off" => b = b.no_proxy(),
            "system" | "" => {} // reqwest reads HTTP(S)_PROXY / ALL_PROXY / NO_PROXY and OS settings
            url => {
                b = b.proxy(
                    reqwest::Proxy::all(url).with_context(|| format!("代理地址无效: {url}"))?,
                )
            }
        }
        Ok(Self {
            client: b.build()?,
            settings: settings.clone(),
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn mirror_mode(&self) -> &str {
        &self.settings.mirror
    }

    /// Apply mirror rules to an upstream URL.
    pub fn rewrite(&self, url: &str) -> String {
        self.settings.rewrite_url(url)
    }

    fn get(&self, url: &str) -> Result<reqwest::blocking::Response> {
        let real = self.rewrite(url);
        let mut req = self.client.get(&real).timeout(Duration::from_secs(120));
        if real.starts_with("https://api.github.com/") {
            if let Ok(tok) = std::env::var("GITHUB_TOKEN") {
                if !tok.is_empty() {
                    req = req.bearer_auth(tok);
                }
            }
        }
        let resp = req.send().with_context(|| format!("请求失败: {real}"))?;
        if !resp.status().is_success() {
            bail!("请求 {real} 返回 HTTP {}", resp.status());
        }
        Ok(resp)
    }

    pub fn get_text(&self, url: &str) -> Result<String> {
        Ok(self.get(url)?.text()?)
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let text = self.get_text(url)?;
        serde_json::from_str(&text).with_context(|| format!("解析 JSON 失败: {url}"))
    }

    /// Download `url` to `dest` (via a `.part` file). Verifies SHA-256 when `sha256` is given.
    /// `progress(downloaded, total)` is called periodically.
    pub fn download(
        &self,
        url: &str,
        dest: &Path,
        sha256: Option<&str>,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        let real = self.rewrite(url);
        let resp = self
            .client
            .get(&real)
            .send()
            .with_context(|| format!("下载失败: {real}"))?;
        if !resp.status().is_success() {
            bail!("下载 {real} 返回 HTTP {}", resp.status());
        }
        let total = resp.content_length();
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let part = dest.with_extension("part");
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut reader = resp;
        let mut buf = vec![0u8; 256 * 1024];
        let mut done: u64 = 0;
        progress(0, total);
        loop {
            let n = reader
                .read(&mut buf)
                .with_context(|| format!("读取下载流失败: {real}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(done, total);
        }
        file.flush()?;
        drop(file);
        if let Some(t) = total {
            if done != t {
                let _ = std::fs::remove_file(&part);
                bail!("下载不完整: {done}/{t} 字节");
            }
        }
        if let Some(expected) = sha256 {
            let actual = hex::encode(hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected.trim()) {
                let _ = std::fs::remove_file(&part);
                return Err(anyhow!(
                    "SHA256 校验失败\n  期望: {expected}\n  实际: {actual}"
                ));
            }
        }
        std::fs::rename(&part, dest)?;
        Ok(())
    }
}

/// SHA-256 of a file on disk.
pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Find the hash for `file_name` in a `SHASUMS256.txt` / `SHA256SUMS` style listing.
pub fn find_in_shasums(text: &str, file_name: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == file_name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shasums_lookup() {
        let t = "aaaa  x.tar.gz\n0123456789012345678901234567890123456789012345678901234567890123  node-v20.0.0-linux-x64.tar.gz\n";
        assert_eq!(
            find_in_shasums(t, "node-v20.0.0-linux-x64.tar.gz").unwrap(),
            "0123456789012345678901234567890123456789012345678901234567890123"
        );
        assert!(find_in_shasums(t, "x.tar.gz").is_none()); // bad hash length
        assert!(find_in_shasums(t, "missing").is_none());
    }

    #[test]
    fn sha256_of_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("f");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
