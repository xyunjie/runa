//! Tiny persistence layer: atomic JSON writes plus an advisory lock file, so the CLI and
//! the GUI can update the same files concurrently. Kept deliberately small so it can be
//! swapped for a real database later without touching callers.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// A lock older than this is considered abandoned (crashed writer) and is broken.
const STALE_LOCK: Duration = Duration::from_secs(30);
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// Exclusive lock held while the guard lives (`<file>.lock`, created with O_EXCL).
pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    pub fn acquire(target: &Path) -> Result<Self> {
        let path = lock_path(target);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let start = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .map(|age| age > STALE_LOCK)
                        .unwrap_or(false);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > LOCK_WAIT {
                        anyhow::bail!("等待文件锁超时: {}", path.display());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("无法创建锁文件 {}", path.display()))
                }
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn lock_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    target.with_file_name(name)
}

/// Write `bytes` to `path` atomically: temp file in the same directory, fsync, rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(path.file_name().unwrap_or_default());
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let tmp = dir.join(tmp_name);
    let res = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.with_context(|| format!("写入 {} 失败", path.display()))
}

/// Serialize `value` and write it atomically under the file lock.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let _lock = FileLock::acquire(path)?;
    write_atomic(path, &bytes)
}

/// Read a JSON file; `None` when missing or unparsable (callers treat both as "no data").
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let t = std::fs::read(path).ok()?;
    serde_json::from_slice(&t).ok()
}

/// Remove a file under the lock (missing is fine).
pub fn remove(path: &Path) -> Result<()> {
    let _lock = FileLock::acquire(path)?;
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("sub").join("a.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("a.json")]);
    }

    #[test]
    fn json_roundtrip_and_lock_released() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x.json");
        write_json(&p, &vec![1, 2, 3]).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p), Some(vec![1, 2, 3]));
        assert!(!lock_path(&p).exists());
        {
            let _l = FileLock::acquire(&p).unwrap();
            assert!(lock_path(&p).exists());
        }
        assert!(!lock_path(&p).exists());
        remove(&p).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p), None);
    }

    #[test]
    fn concurrent_writers_never_corrupt() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("c.json");
        let hs: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        write_json(&p, &vec![i; 100]).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let v: Vec<i32> = read_json(&p).unwrap();
        assert_eq!(v.len(), 100);
    }
}
