//! Per-project environments (`runa project init`).
//!
//! A project is a directory containing `.runa/project.json`. Three isolation modes:
//! - `local` (default): installs, shims and caches live in `<project>/.runa/env`, so the
//!   project never touches the user's global runtimes;
//! - `versions`: only records that the directory is a project; uses the global installs;
//! - `docker`: `runa run` executes commands inside an explicit Docker image.
//!
//! `RUNA_PROJECT_BOUNDARY` (set by the MCP server) forbids walking above a bound root.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::resolver::{self, Spec};

pub const DIR: &str = ".runa";
pub const FILE: &str = "project.json";
pub const BOUNDARY_VAR: &str = "RUNA_PROJECT_BOUNDARY";
pub const MODES: &[&str] = &["local", "versions", "docker"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Filled in when loading; never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
}

fn valid_image(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "._/:@-".contains(c))
}

impl Project {
    pub fn new(mode: &str, image: Option<&str>) -> Result<Self> {
        let p = Project {
            schema_version: 1,
            mode: mode.to_string(),
            image: image.filter(|s| !s.is_empty()).map(str::to_string),
            root: None,
        };
        p.validate()?;
        Ok(p)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "不支持的项目环境版本 schema_version={}",
                self.schema_version
            );
        }
        match self.mode.as_str() {
            "local" | "versions" => {
                if self.image.is_some() {
                    bail!("--image 只能用于 docker 模式");
                }
            }
            "docker" => match &self.image {
                Some(i) if valid_image(i) => {}
                _ => bail!("docker 模式需要明确的 --image（例如 node:22-bookworm）"),
            },
            other => bail!("未知项目模式 {other:?}（可选 local / versions / docker）"),
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        self.root.as_deref().unwrap_or(Path::new("."))
    }

    /// `<project>/.runa/env` for isolated modes.
    pub fn env_root(&self) -> PathBuf {
        self.root().join(DIR).join("env")
    }

    pub fn isolated(&self) -> bool {
        self.mode != "versions"
    }

    /// Paths used for this project, given the user's global paths.
    pub fn paths(&self, global: &Paths) -> Paths {
        if self.isolated() {
            global.isolated(self.env_root())
        } else {
            global.clone()
        }
    }

    /// Variables that keep package-manager state inside the project (local mode only).
    pub fn variables(&self) -> Vec<(String, String)> {
        if self.mode != "local" {
            return vec![];
        }
        let base = self.env_root();
        let p = |rel: &[&str]| {
            let mut b = base.clone();
            for r in rel {
                b.push(r);
            }
            b.display().to_string()
        };
        vec![
            (
                "RUNA_PROJECT_ROOT".into(),
                self.root().display().to_string(),
            ),
            ("GOTOOLCHAIN".into(), "local".into()),
            ("GOPATH".into(), p(&["go"])),
            ("GOBIN".into(), p(&["go", "bin"])),
            ("GOMODCACHE".into(), p(&["cache", "go-mod"])),
            ("GOCACHE".into(), p(&["cache", "go-build"])),
            ("npm_config_cache".into(), p(&["cache", "npm"])),
            ("npm_config_prefix".into(), p(&["node"])),
            ("npm_config_userconfig".into(), p(&["npmrc"])),
            ("PIP_CACHE_DIR".into(), p(&["cache", "pip"])),
            ("PIP_REQUIRE_VIRTUALENV".into(), "true".into()),
            ("PYTHONNOUSERSITE".into(), "1".into()),
            ("CARGO_HOME".into(), p(&["cargo"])),
            ("GRADLE_USER_HOME".into(), p(&["gradle"])),
        ]
    }

    /// Variables cleared for local projects so the user's global setup cannot leak in.
    pub fn cleared_variables(&self) -> &'static [&'static str] {
        if self.mode == "local" {
            &["PYTHONHOME", "PYTHONPATH", "VIRTUAL_ENV", "NODE_PATH"]
        } else {
            &[]
        }
    }

    /// Create the directories referenced by [`Project::variables`].
    pub fn prepare(&self) -> Result<()> {
        self.validate_state_root()?;
        for (k, v) in self.variables() {
            if k == "RUNA_PROJECT_ROOT" || k == "npm_config_userconfig" || !v.contains(['/', '\\'])
            {
                continue;
            }
            std::fs::create_dir_all(&v).with_context(|| format!("创建 {v} 失败"))?;
        }
        Ok(())
    }

    /// Refuse symlinked state directories (a link could redirect installs outside the project).
    pub fn validate_state_root(&self) -> Result<()> {
        for rel in [PathBuf::from(DIR), Path::new(DIR).join("env")] {
            let p = self.root().join(&rel);
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                    bail!("项目环境目录不能是链接或文件: {}", p.display())
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// Canonical directory, rejecting files.
pub fn project_directory(dir: &Path) -> Result<PathBuf> {
    let abs = dunce(
        std::fs::canonicalize(dir).with_context(|| format!("目录不存在: {}", dir.display()))?,
    );
    if !abs.is_dir() {
        bail!("项目必须是目录: {}", abs.display());
    }
    Ok(abs)
}

/// Strip the `\\?\` prefix Windows' canonicalize adds.
fn dunce(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.display().to_string();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            if !rest.starts_with("UNC") {
                return PathBuf::from(rest);
            }
        }
    }
    p
}

fn within(root: &Path, p: &Path) -> bool {
    let norm = |x: &Path| dunce(std::fs::canonicalize(x).unwrap_or_else(|_| x.to_path_buf()));
    norm(p).starts_with(norm(root))
}

/// Initialize a project. Re-running with the same settings is a no-op; changing the mode
/// of an existing project is refused so an environment is never silently replaced.
pub fn init(dir: &Path, mode: &str, image: Option<&str>) -> Result<Project> {
    let root = project_directory(dir)?;
    let mut p = Project::new(mode, image)?;
    p.root = Some(root.clone());
    p.validate_state_root()?;
    let d = root.join(DIR);
    std::fs::create_dir_all(&d)?;
    let file = d.join(FILE);
    if let Ok(text) = std::fs::read_to_string(&file) {
        let existing: Project = serde_json::from_str(&text).context("project.json 无效")?;
        existing.validate()?;
        if existing.mode != p.mode || existing.image != p.image {
            bail!(
                "该项目已使用 {} 模式，拒绝替换其环境（如需更改请先删除 {}）",
                existing.mode,
                file.display()
            );
        }
        return Ok(p);
    }
    let mut stored = p.clone();
    stored.root = None;
    let tmp = d.join(".project.json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&stored)? + "\n")?;
    std::fs::rename(&tmp, &file)?;
    let ignore = d.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "*\n!.gitignore\n!project.json\n")?;
    }
    Ok(p)
}

/// Find the project containing `start` (walking up). Honors [`BOUNDARY_VAR`].
pub fn find(start: &Path) -> Result<Option<Project>> {
    let boundary = std::env::var_os(BOUNDARY_VAR)
        .filter(|b| !b.is_empty())
        .map(PathBuf::from);
    find_with_boundary(start, boundary.as_deref())
}

pub fn find_with_boundary(start: &Path, boundary: Option<&Path>) -> Result<Option<Project>> {
    let start = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
    if let Some(b) = boundary {
        if !within(b, &start) {
            bail!("目录 {} 不在绑定的项目 {} 内", start.display(), b.display());
        }
    }
    for dir in start.ancestors() {
        let file = dir.join(DIR).join(FILE);
        if let Ok(meta) = std::fs::metadata(&file) {
            if meta.len() > 64 * 1024 {
                bail!("{} 过大", file.display());
            }
            let text = std::fs::read_to_string(&file)?;
            let mut p: Project = serde_json::from_str(&text)
                .with_context(|| format!("解析 {} 失败", file.display()))?;
            p.validate()?;
            p.root = Some(dunce(
                std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf()),
            ));
            return Ok(Some(p));
        }
        if let Some(b) = boundary {
            if within(dir, b) {
                // reached the bound root
                return Ok(None);
            }
        }
    }
    Ok(None)
}

/// What the project files (not env vars, not the global file) ask for.
pub fn requirement(tool: &str, cwd: &Path) -> Option<Spec> {
    let none = |_: &str| None;
    resolver::find_spec(tool, cwd, &none, Path::new(""))
}

/// `docker run` arguments for running `command` in the project's image.
pub fn docker_args(p: &Project, dir: &Path, command: &str, args: &[String]) -> Result<Vec<String>> {
    p.validate()?;
    let image = p
        .image
        .clone()
        .ok_or_else(|| anyhow!("docker 模式缺少镜像"))?;
    if command.is_empty() || command.contains(['/', '\\', '\0', '\n', '\r']) {
        bail!("命令必须是命令名: {command:?}");
    }
    let root = p.root();
    let rel = dir
        .strip_prefix(root)
        .map_err(|_| anyhow!("Docker 工作目录必须在项目内"))?;
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");
    let root_s = root.display().to_string();
    if root_s.contains([',', '"']) {
        bail!("项目路径包含 Docker 挂载不支持的字符: {root_s}");
    }
    let mut v: Vec<String> = [
        "run",
        "--rm",
        "--init",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pids-limit=256",
        "--read-only",
        "--tmpfs",
        "/tmp:rw,nosuid,nodev",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some((uid, gid)) = ids() {
        v.push("--user".into());
        v.push(format!("{uid}:{gid}"));
    }
    v.push("--mount".into());
    v.push(format!("type=bind,src={root_s},dst=/workspace"));
    v.push("--workdir".into());
    v.push(if rel.is_empty() {
        "/workspace".into()
    } else {
        format!("/workspace/{rel}")
    });
    if let Ok(id) = std::env::var("RUNA_MCP_REQUEST_ID") {
        if !id.is_empty() {
            v.push("--label".into());
            v.push(format!("io.github.runa.mcp={id}"));
        }
    }
    for e in [
        "GOTOOLCHAIN=local",
        "PYTHONNOUSERSITE=1",
        "HOME=/workspace/.runa/docker-cache/home",
        "GOPATH=/workspace/.runa/docker-cache/go",
        "GOCACHE=/workspace/.runa/docker-cache/go-build",
        "GOMODCACHE=/workspace/.runa/docker-cache/go-mod",
        "npm_config_cache=/workspace/.runa/docker-cache/npm",
        "PIP_CACHE_DIR=/workspace/.runa/docker-cache/pip",
    ] {
        v.push("--env".into());
        v.push(e.into());
    }
    v.push(image);
    v.push(command.into());
    v.extend(args.iter().cloned());
    Ok(v)
}

#[cfg(unix)]
fn ids() -> Option<(u32, u32)> {
    let out = std::process::Command::new("id").arg("-u").output().ok()?;
    let gid = std::process::Command::new("id").arg("-g").output().ok()?;
    let u = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    let g = String::from_utf8_lossy(&gid.stdout).trim().parse().ok()?;
    Some((u, g))
}

#[cfg(not(unix))]
fn ids() -> Option<(u32, u32)> {
    None
}

pub fn docker_cache_dirs(p: &Project) -> Result<()> {
    for n in ["home", "go", "go-build", "go-mod", "npm", "pip"] {
        std::fs::create_dir_all(p.root().join(DIR).join("docker-cache").join(n))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_modes() {
        assert!(Project::new("local", None).is_ok());
        assert!(Project::new("versions", None).is_ok());
        assert!(Project::new("local", Some("node:22")).is_err());
        assert!(Project::new("docker", None).is_err());
        assert!(Project::new("docker", Some("-rm")).is_err());
        assert!(Project::new("docker", Some("node:22 --privileged")).is_err());
        assert!(Project::new("docker", Some("ghcr.io/a/b:1@sha256:ab")).is_ok());
        assert!(Project::new("vm", None).is_err());
    }

    #[test]
    fn init_find_and_refuse_mode_change() {
        let t = tempfile::tempdir().unwrap();
        let p = init(t.path(), "local", None).unwrap();
        assert_eq!(p.mode, "local");
        let stored = std::fs::read_to_string(t.path().join(".runa/project.json")).unwrap();
        assert!(!stored.contains("root"));
        assert!(t.path().join(".runa/.gitignore").is_file());
        // idempotent
        init(t.path(), "local", None).unwrap();
        assert!(init(t.path(), "versions", None).is_err());
        let sub = t.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        let found = find_with_boundary(&sub, None).unwrap().unwrap();
        assert_eq!(
            found.root.unwrap(),
            dunce(std::fs::canonicalize(t.path()).unwrap())
        );
    }

    #[test]
    fn boundary_is_enforced() {
        let t = tempfile::tempdir().unwrap();
        let inner = t.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        init(t.path(), "local", None).unwrap();
        // the project above the boundary is not visible
        assert!(find_with_boundary(&inner, Some(&inner)).unwrap().is_none());
        // escaping the boundary is an error
        assert!(find_with_boundary(t.path(), Some(&inner)).is_err());
    }

    #[test]
    fn isolated_paths_keep_home() {
        let t = tempfile::tempdir().unwrap();
        let p = init(t.path(), "local", None).unwrap();
        let g = Paths::with_root("/home/u/.runa");
        let pp = p.paths(&g);
        assert!(pp.root.ends_with(".runa/env"));
        assert_eq!(pp.settings_file(), g.settings_file());
        assert!(pp.is_isolated());
        let mut v = p.clone();
        v.mode = "versions".into();
        assert!(!v.paths(&g).is_isolated());
        assert!(p.variables().iter().any(|(k, _)| k == "GOTOOLCHAIN"));
        assert!(v.variables().is_empty());
    }

    #[test]
    fn symlinked_state_is_refused() {
        #[cfg(unix)]
        {
            let t = tempfile::tempdir().unwrap();
            let other = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(other.path(), t.path().join(".runa")).unwrap();
            assert!(init(t.path(), "local", None).is_err());
        }
    }

    #[test]
    fn docker_arguments() {
        let t = tempfile::tempdir().unwrap();
        let mut p = init(t.path(), "docker", Some("node:22")).unwrap();
        let root = p.root.clone().unwrap();
        let args = docker_args(&p, &root.join("src"), "node", &["-v".into()]).unwrap();
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--cap-drop=ALL".to_string()));
        assert!(args.contains(&"/workspace/src".to_string()));
        assert_eq!(&args[args.len() - 3..], &["node:22", "node", "-v"]);
        assert!(docker_args(&p, &root, "../x", &[]).is_err());
        assert!(docker_args(&p, Path::new("/elsewhere"), "node", &[]).is_err());
        p.image = Some("bad image".into());
        assert!(docker_args(&p, &root, "node", &[]).is_err());
    }
}
