//! Self-update from GitHub Releases (`runa update`).
//!
//! Release assets are produced by `.github/workflows/release.yml`:
//! `runa-<tag>-<target>.tar.gz` (`.zip` on Windows) plus a `SHA256SUMS` listing.
//! The SHA-256 comes from GitHub's per-asset `digest` when present, otherwise from
//! `SHA256SUMS`; an update without a publisher checksum is refused. The new binary is
//! staged next to the current one, probed with `--version`, and swapped in with a backup
//! that is restored on failure. Only stable releases newer than the running one apply.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::archive;
use crate::net::{find_in_shasums, Net};
use crate::version;

pub const DEFAULT_REPO: &str = "xyunjie/runa";
const MAX_ASSET: u64 = 256 << 20;

/// `owner/name`, overridable with `RUNA_UPDATE_REPO` (forks).
pub fn repo() -> String {
    std::env::var("RUNA_UPDATE_REPO")
        .ok()
        .filter(|r| valid_repo(r))
        .unwrap_or_else(|| DEFAULT_REPO.to_string())
}

fn valid_repo(r: &str) -> bool {
    let parts: Vec<&str> = r.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && !p.starts_with('.')
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub repo: String,
    pub current: String,
    pub latest: String,
    pub available: bool,
    pub url: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}

/// `v1.2.3` / `1.2.3` only.
pub fn is_stable(v: &str) -> bool {
    let v = v.strip_prefix('v').unwrap_or(v);
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Validate release metadata and build [`Info`].
pub fn info_from_release(
    repo: &str,
    current: &str,
    wanted: Option<&str>,
    r: Release,
) -> Result<Info> {
    if r.draft || r.prerelease || !is_stable(&r.tag_name) {
        bail!("仅支持已发布的稳定版本（得到 {}）", r.tag_name);
    }
    let tag = format!("v{}", r.tag_name.trim_start_matches('v'));
    if let Some(w) = wanted {
        if w.trim_start_matches('v') != tag.trim_start_matches('v') {
            bail!("GitHub 返回了其它版本 {tag}");
        }
    }
    if r.assets.is_empty() {
        bail!("{tag} 没有可下载的安装包");
    }
    Ok(Info {
        repo: repo.into(),
        current: current.into(),
        available: version::compare(tag.trim_start_matches('v'), current.trim_start_matches('v'))
            == std::cmp::Ordering::Greater,
        url: r
            .html_url
            .unwrap_or_else(|| format!("https://github.com/{repo}/releases/tag/{}", r.tag_name)),
        notes: r.body.unwrap_or_default(),
        latest: tag,
        assets: r.assets,
    })
}

/// Query the latest release, or a specific stable `version`.
pub fn check(net: &Net, current: &str, version: Option<&str>) -> Result<Info> {
    let repo = repo();
    let url = match version {
        Some(v) => {
            if !is_stable(v) {
                bail!("更新版本必须是稳定版本号，例如 v1.0.5");
            }
            format!(
                "https://api.github.com/repos/{repo}/releases/tags/v{}",
                v.trim_start_matches('v')
            )
        }
        None => format!("https://api.github.com/repos/{repo}/releases/latest"),
    };
    let release: Release = match net.get_json(&url) {
        Ok(r) => r,
        Err(e) if format!("{e:#}").contains("404") => {
            bail!(
                "{repo} 还没有{}稳定发布版本",
                version.map(|v| format!(" {v} 这个")).unwrap_or_default()
            )
        }
        Err(e) => return Err(e.context(format!("读取 {repo} 的 GitHub Release 失败"))),
    };
    info_from_release(&repo, current, version, release)
}

/// Rust target triple of the running binary, as used in release asset names.
pub fn target_triple() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    })
}

pub fn asset_name(tag: &str, triple: &str) -> String {
    let ext = if triple.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("runa-{tag}-{triple}.{ext}")
}

impl Info {
    /// The single asset called `name`, served from this repo's release download URL.
    pub fn asset(&self, name: &str) -> Result<&Asset> {
        let mut found = self.assets.iter().filter(|a| a.name == name);
        let a = found
            .next()
            .ok_or_else(|| anyhow!("{} 没有 {name}；请到 {} 手动下载", self.latest, self.url))?;
        if found.next().is_some() {
            bail!("重复的更新资源 {name}");
        }
        let want = format!(
            "https://github.com/{}/releases/download/{}/{name}",
            self.repo, self.latest
        );
        if a.browser_download_url != want || a.size == 0 || a.size > MAX_ASSET {
            bail!("更新资源来源或大小无效: {name}");
        }
        Ok(a)
    }
}

/// `sha256:<64 hex>` -> lowercase hex.
pub fn digest_value(v: &str) -> Result<String> {
    let hex = v
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("缺少 SHA256 摘要"))?;
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("无效的 SHA256 摘要");
    }
    Ok(hex.to_ascii_lowercase())
}

/// Publisher checksum for `name`: asset digest first, then `SHA256SUMS`.
pub fn expected_sha256(
    info: &Info,
    name: &str,
    shasums: &dyn Fn(&Asset) -> Result<String>,
) -> Result<String> {
    let a = info.asset(name)?;
    if let Some(d) = &a.digest {
        return digest_value(d);
    }
    let sums = info
        .asset("SHA256SUMS")
        .map_err(|_| anyhow!("发布方未提供 {name} 的 SHA256；停止更新"))?;
    let text = shasums(sums)?;
    let mut hits = text.lines().filter(|l| {
        l.split_whitespace()
            .nth(1)
            .map(|n| n.trim_start_matches('*') == name)
            .unwrap_or(false)
    });
    if hits.next().is_some() && hits.next().is_some() {
        bail!("SHA256SUMS 中 {name} 有重复记录");
    }
    find_in_shasums(&text, name).ok_or_else(|| anyhow!("发布方未提供 {name} 的 SHA256；停止更新"))
}

#[derive(Debug, Clone, Serialize)]
pub struct Applied {
    pub version: String,
    pub path: PathBuf,
}

/// Download, verify and install `info.latest` over `exe`.
pub fn apply(
    net: &Net,
    info: &Info,
    exe: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Applied> {
    if !is_stable(&info.latest) || !info.available {
        bail!(
            "目标版本 {} 不比当前版本 {} 新，拒绝降级或重复替换",
            info.latest,
            info.current
        );
    }
    let triple = target_triple().ok_or_else(|| anyhow!("当前平台没有预编译的 runa 发布包"))?;
    let name = asset_name(&info.latest, triple);
    let sha = expected_sha256(info, &name, &|a| net.get_text(&a.browser_download_url))?;
    let asset = info.asset(&name)?;

    let exe = std::fs::canonicalize(exe).with_context(|| format!("找不到 {}", exe.display()))?;
    let meta = std::fs::symlink_metadata(&exe)?;
    if !meta.is_file() {
        bail!("更新目标必须是普通文件: {}", exe.display());
    }
    let dir = exe.parent().ok_or_else(|| anyhow!("无效的程序路径"))?;
    let lock = dir.join(".runa-update.lock");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .with_context(|| format!("已有更新任务，或目录不可写: {}", dir.display()))?;
    let stage = dir.join(format!(".runa-update-{}", std::process::id()));
    let res = (|| -> Result<Applied> {
        std::fs::create_dir_all(&stage)?;
        let archive_path = stage.join(&name);
        net.download(
            &asset.browser_download_url,
            &archive_path,
            Some(&sha),
            progress,
        )?;
        let out = stage.join("x");
        archive::extract(&archive_path, &out)?;
        let bin_name = crate::platform::exe_name("runa");
        let new_bin =
            find_file(&out, &bin_name, 3).ok_or_else(|| anyhow!("发布包中没有 {bin_name}"))?;
        probe(&new_bin, &info.latest)?;
        replace(&exe, &new_bin)?;
        Ok(Applied {
            version: info.latest.clone(),
            path: exe.clone(),
        })
    })();
    let _ = std::fs::remove_dir_all(&stage);
    let _ = std::fs::remove_file(&lock);
    res
}

fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subdirs = vec![];
    for e in rd.flatten() {
        let p = e.path();
        let ft = e.file_type().ok()?;
        if ft.is_file() && e.file_name() == name {
            return Some(p);
        }
        if ft.is_dir() {
            subdirs.push(p);
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs
        .into_iter()
        .find_map(|d| find_file(&d, name, depth - 1))
}

/// The staged binary must run and report exactly the release version.
fn probe(bin: &Path, tag: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755))?;
    }
    let out = std::process::Command::new(bin)
        .arg("--version")
        .env("RUNA_DISABLE_SHIM", "1")
        .output()
        .with_context(|| format!("无法运行新版本 {}", bin.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let want = format!("runa {}", tag.trim_start_matches('v'));
    if !out.status.success() || text.trim() != want {
        bail!("新版本自检失败：期望 `{want}`，得到 `{}`", text.trim());
    }
    Ok(())
}

/// Swap `new` into `target`, keeping a backup until the swap succeeds.
pub fn replace(target: &Path, new: &Path) -> Result<()> {
    let backup = target.with_file_name(format!(
        "{}.old-{}",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("runa"),
        std::process::id()
    ));
    std::fs::rename(target, &backup)
        .with_context(|| format!("无法备份旧程序 {}", target.display()))?;
    if let Err(e) = std::fs::rename(new, target).or_else(|_| std::fs::copy(new, target).map(|_| ()))
    {
        let _ = std::fs::remove_file(target);
        std::fs::rename(&backup, target)
            .with_context(|| format!("替换失败且回滚失败，旧程序保留在 {}", backup.display()))?;
        return Err(anyhow!("替换 {} 失败: {e}", target.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755));
    }
    // A running .exe cannot be deleted on Windows; the backup is cleaned next time.
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(repo: &str, tag: &str, name: &str, digest: Option<&str>) -> Asset {
        Asset {
            name: name.into(),
            browser_download_url: format!(
                "https://github.com/{repo}/releases/download/{tag}/{name}"
            ),
            size: 10,
            digest: digest.map(str::to_string),
        }
    }

    fn release(tag: &str, assets: Vec<Asset>) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: false,
            body: None,
            html_url: None,
            assets,
        }
    }

    #[test]
    fn stable_versions() {
        assert!(is_stable("v1.2.3"));
        assert!(is_stable("0.1.0"));
        assert!(!is_stable("v1.2"));
        assert!(!is_stable("v1.2.3-rc1"));
        assert!(!is_stable("latest"));
    }

    #[test]
    fn release_validation() {
        let r = release("v0.2.0", vec![asset("a/b", "v0.2.0", "x", None)]);
        let i = info_from_release("a/b", "0.1.0", None, r.clone()).unwrap();
        assert!(i.available);
        assert_eq!(i.latest, "v0.2.0");
        assert!(
            !info_from_release("a/b", "0.2.0", None, r.clone())
                .unwrap()
                .available
        );
        assert!(info_from_release("a/b", "0.1.0", Some("0.3.0"), r.clone()).is_err());
        let mut pre = r.clone();
        pre.prerelease = true;
        assert!(info_from_release("a/b", "0.1.0", None, pre).is_err());
        let mut draft = r.clone();
        draft.draft = true;
        assert!(info_from_release("a/b", "0.1.0", None, draft).is_err());
        assert!(info_from_release("a/b", "0.1.0", None, release("v0.2.0", vec![])).is_err());
    }

    #[test]
    fn asset_origin_is_checked() {
        let name = asset_name("v0.2.0", "x86_64-unknown-linux-gnu");
        assert_eq!(name, "runa-v0.2.0-x86_64-unknown-linux-gnu.tar.gz");
        assert!(asset_name("v1.0.0", "x86_64-pc-windows-msvc").ends_with(".zip"));
        let mut bad = asset("evil/fork", "v0.2.0", &name, None);
        bad.name = name.clone();
        let i = info_from_release("a/b", "0.1.0", None, release("v0.2.0", vec![bad])).unwrap();
        assert!(i.asset(&name).is_err());
        let dup = vec![
            asset("a/b", "v0.2.0", &name, None),
            asset("a/b", "v0.2.0", &name, None),
        ];
        let i = info_from_release("a/b", "0.1.0", None, release("v0.2.0", dup)).unwrap();
        assert!(i.asset(&name).is_err());
    }

    #[test]
    fn checksum_sources() {
        let h = "a".repeat(64);
        let name = "runa-v0.2.0-x86_64-unknown-linux-gnu.tar.gz";
        let no_sums = |_: &Asset| -> Result<String> { panic!("should not fetch") };
        // GitHub digest wins
        let i = info_from_release(
            "a/b",
            "0.1.0",
            None,
            release(
                "v0.2.0",
                vec![asset("a/b", "v0.2.0", name, Some(&format!("sha256:{h}")))],
            ),
        )
        .unwrap();
        assert_eq!(expected_sha256(&i, name, &no_sums).unwrap(), h);
        // SHA256SUMS fallback
        let i = info_from_release(
            "a/b",
            "0.1.0",
            None,
            release(
                "v0.2.0",
                vec![
                    asset("a/b", "v0.2.0", name, None),
                    asset("a/b", "v0.2.0", "SHA256SUMS", None),
                ],
            ),
        )
        .unwrap();
        let sums = |_: &Asset| Ok(format!("{h}  {name}\n{}  other\n", "b".repeat(64)));
        assert_eq!(expected_sha256(&i, name, &sums).unwrap(), h);
        let dup = |_: &Asset| Ok(format!("{h}  {name}\n{h}  {name}\n"));
        assert!(expected_sha256(&i, name, &dup).is_err());
        let missing = |_: &Asset| Ok("".to_string());
        assert!(expected_sha256(&i, name, &missing).is_err());
        // no checksum at all
        let i = info_from_release(
            "a/b",
            "0.1.0",
            None,
            release("v0.2.0", vec![asset("a/b", "v0.2.0", name, None)]),
        )
        .unwrap();
        assert!(expected_sha256(&i, name, &no_sums).is_err());
        assert!(digest_value("md5:abc").is_err());
        assert!(digest_value("sha256:xyz").is_err());
    }

    #[test]
    fn replace_swaps_file() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("runa");
        let new = t.path().join("new");
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        replace(&target, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 1);
        // failed swap restores the original
        assert!(replace(&target, &t.path().join("missing")).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    }

    #[test]
    fn repo_override_is_validated() {
        assert!(valid_repo("xyunjie/runa"));
        assert!(!valid_repo("../x"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a b/c"));
    }
}
