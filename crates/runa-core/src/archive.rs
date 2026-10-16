//! Archive extraction with zip-slip protection.
//!
//! Every entry path must be relative and free of `..`; symlink targets must stay inside
//! the destination. After extraction a single top-level directory is flattened so that
//! `installs/<tool>/<version>/bin` exists directly.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    TarGz,
    Zip,
}

pub fn kind_of(name: &str) -> Option<Kind> {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".tar.gz") || l.ends_with(".tgz") {
        Some(Kind::TarGz)
    } else if l.ends_with(".zip") {
        Some(Kind::Zip)
    } else {
        None
    }
}

/// Validate an archive entry path and return it as a safe relative path.
pub fn safe_relative(p: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("压缩包包含不安全路径（zip-slip）: {}", p.display())
            }
        }
    }
    Ok(out)
}

/// Lexically normalise `base.join(rel)` and check it stays inside `root`.
pub fn link_target_inside(root: &Path, link_rel: &Path, target: &Path) -> bool {
    if target.is_absolute() || target.has_root() {
        return false;
    }
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    if let Some(parent) = link_rel.parent() {
        for c in parent.components() {
            if let Component::Normal(s) = c {
                stack.push(s.to_os_string());
            }
        }
    }
    for c in target.components() {
        match c {
            Component::Normal(s) => stack.push(s.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    let _ = root;
    true
}

pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("");
    fs::create_dir_all(dest)?;
    match kind_of(name) {
        Some(Kind::TarGz) => extract_tar_gz(archive, dest),
        Some(Kind::Zip) => extract_zip(archive, dest),
        None => bail!("不支持的压缩格式: {name}"),
    }
    .with_context(|| format!("解压 {} 失败", archive.display()))
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let f = fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(io::BufReader::new(f));
    let mut ar = tar::Archive::new(gz);
    ar.set_preserve_permissions(true);
    ar.set_overwrite(true);
    for entry in ar.entries()? {
        let mut entry = entry?;
        let raw = entry.path()?.into_owned();
        let rel = safe_relative(&raw)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let et = entry.header().entry_type();
        if et.is_symlink() || et.is_hard_link() {
            let target = entry
                .link_name()?
                .map(|t| t.into_owned())
                .unwrap_or_default();
            let ok = if et.is_hard_link() {
                safe_relative(&target).is_ok()
            } else {
                link_target_inside(dest, &rel, &target)
            };
            if !ok {
                bail!(
                    "压缩包中的链接指向目标目录之外: {} -> {}",
                    rel.display(),
                    target.display()
                );
            }
        }
        // tar::Entry::unpack_in additionally refuses to write through symlinks escaping `dest`.
        if !entry.unpack_in(dest)? {
            bail!("压缩包包含不安全路径: {}", raw.display());
        }
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let f = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(f))?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let raw = PathBuf::from(file.name());
        let rel = match file.enclosed_name() {
            Some(p) => safe_relative(&p)?,
            None => bail!("压缩包包含不安全路径（zip-slip）: {}", raw.display()),
        };
        safe_relative(&raw)?;
        let out = dest.join(&rel);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p)?;
        }
        let mut w = fs::File::create(&out)?;
        io::copy(&mut file, &mut w)?;
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o7777))?;
        }
    }
    Ok(())
}

/// If `dir` contains exactly one directory and nothing else, move its contents up.
/// Returns the directory that now holds the payload (always `dir`).
pub fn flatten_single_root(dir: &Path) -> Result<()> {
    let entries: Vec<_> = fs::read_dir(dir)?.collect::<std::result::Result<_, _>>()?;
    if entries.len() != 1 {
        return Ok(());
    }
    let only = entries[0].path();
    if !fs::symlink_metadata(&only)?.is_dir() {
        return Ok(());
    }
    let tmp = dir.join(".__runa_flatten__");
    fs::rename(&only, &tmp)?;
    for e in fs::read_dir(&tmp)? {
        let e = e?;
        fs::rename(e.path(), dir.join(e.file_name()))?;
    }
    fs::remove_dir(&tmp)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rejects_traversal() {
        assert!(safe_relative(Path::new("../evil")).is_err());
        assert!(safe_relative(Path::new("a/../../evil")).is_err());
        assert!(safe_relative(Path::new("/etc/passwd")).is_err());
        assert_eq!(
            safe_relative(Path::new("./a/b")).unwrap(),
            PathBuf::from("a/b")
        );
    }

    #[test]
    fn symlink_targets() {
        let root = Path::new("/x");
        assert!(link_target_inside(
            root,
            Path::new("bin/npm"),
            Path::new("../lib/npm-cli.js")
        ));
        assert!(!link_target_inside(
            root,
            Path::new("bin/npm"),
            Path::new("../../etc/passwd")
        ));
        assert!(!link_target_inside(
            root,
            Path::new("npm"),
            Path::new("/etc/passwd")
        ));
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            z.start_file(*name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn zip_extract_and_flatten() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("t.zip");
        make_zip(&a, &[("pkg-1.0/bin/tool", b"hi"), ("pkg-1.0/README", b"r")]);
        let d = tmp.path().join("out");
        extract(&a, &d).unwrap();
        flatten_single_root(&d).unwrap();
        assert_eq!(fs::read(d.join("bin/tool")).unwrap(), b"hi");
        assert!(d.join("README").exists());
    }

    #[test]
    fn zip_slip_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("evil.zip");
        make_zip(&a, &[("../../evil.txt", b"x")]);
        let d = tmp.path().join("out");
        assert!(extract(&a, &d).is_err());
        assert!(!tmp.path().join("evil.txt").exists());
    }

    #[test]
    fn tar_gz_extract_and_reject_escape_link() {
        let tmp = tempfile::tempdir().unwrap();
        // good archive
        let good = tmp.path().join("good.tar.gz");
        {
            let f = fs::File::create(&good).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let mut h = tar::Header::new_gnu();
            h.set_size(2);
            h.set_mode(0o755);
            h.set_cksum();
            b.append_data(&mut h, "node-v1/bin/node", &b"hi"[..])
                .unwrap();
            let mut l = tar::Header::new_gnu();
            l.set_entry_type(tar::EntryType::Symlink);
            l.set_size(0);
            b.append_link(&mut l, "node-v1/bin/npm", "../lib/npm-cli.js")
                .unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        let d = tmp.path().join("out");
        extract(&good, &d).unwrap();
        flatten_single_root(&d).unwrap();
        assert!(d.join("bin/node").exists());

        // archive with a symlink escaping the destination
        let bad = tmp.path().join("bad.tar.gz");
        {
            let f = fs::File::create(&bad).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            let mut l = tar::Header::new_gnu();
            l.set_entry_type(tar::EntryType::Symlink);
            l.set_size(0);
            b.append_link(&mut l, "x/escape", "../../../../etc")
                .unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        assert!(extract(&bad, &tmp.path().join("out2")).is_err());
    }
}
