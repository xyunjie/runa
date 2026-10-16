//! High level operations shared by the CLI and the GUI.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

use crate::installer::{self, Progress, Receipt};
use crate::net::Net;
use crate::paths::Paths;
use crate::platform;
use crate::plugins::{PluginRef, Registry};
use crate::project::{self, Project};
use crate::resolver::{self, Spec};
use crate::settings::Settings;
use crate::toolversions;
use crate::version::{self, VersionReq};

pub struct Manager {
    pub paths: Paths,
    pub settings: Settings,
    pub registry: Registry,
    /// Non fatal problems (e.g. broken manifest plugins).
    pub warnings: Vec<String>,
    /// The project environment this manager is bound to (see [`Manager::load_for`]).
    pub project: Option<Project>,
    /// Refuse installs whose publisher does not provide a SHA-256.
    pub require_checksum: bool,
}

/// The version that applies to a tool in a directory.
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub tool: String,
    pub spec: Spec,
    /// Installed version satisfying the spec, if any.
    pub version: Option<String>,
    /// Spec is `system`: runa injects nothing.
    pub system: bool,
}

impl Resolved {
    pub fn wanted(&self) -> &str {
        self.spec.specs.first().map(|s| s.as_str()).unwrap_or("")
    }
}

/// PATH entries and environment variables for a directory.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EnvPlan {
    pub paths: Vec<PathBuf>,
    pub vars: Vec<(String, String)>,
    pub tools: Vec<Resolved>,
}

/// Per-tool overview (used by `runa list` and the GUI).
#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub tool: String,
    pub description: String,
    pub builtin: bool,
    pub installed: Vec<InstalledVersion>,
    pub global: Option<String>,
    pub current: Option<Resolved>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledVersion {
    pub version: String,
    pub source: String,
    pub path: PathBuf,
    pub origin: Option<PathBuf>,
}

impl Manager {
    pub fn load() -> Result<Self> {
        Self::with_paths(Paths::detect()?)
    }

    /// Load for a working directory: inside an isolated project (`runa project init`),
    /// installs, shims and versions come from `<project>/.runa/env`.
    pub fn load_for(cwd: &Path) -> Result<Self> {
        let global = Paths::detect()?;
        match project::find(cwd)? {
            Some(p) => {
                p.validate_state_root()?;
                let mut m = Self::with_paths(p.paths(&global))?;
                m.project = Some(p);
                Ok(m)
            }
            None => Self::with_paths(global),
        }
    }

    pub fn with_paths(paths: Paths) -> Result<Self> {
        let settings = Settings::load(&paths.settings_file())?;
        let (registry, warnings) = Registry::load(&paths.plugins());
        Ok(Self {
            paths,
            settings,
            registry,
            warnings,
            project: None,
            require_checksum: false,
        })
    }

    pub fn net(&self) -> Result<Net> {
        Net::new(&self.settings)
    }

    pub fn save_settings(&self) -> Result<()> {
        self.settings.save(&self.paths.settings_file())
    }

    pub fn plugin(&self, tool: &str) -> Result<PluginRef> {
        self.registry.get(tool).ok_or_else(|| {
            anyhow!(
                "未知工具: {tool}（可用: {}）",
                self.registry.names().join(", ")
            )
        })
    }

    /// Installed versions of `tool`, newest first.
    pub fn installed(&self, tool: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.paths.tool_dir(tool)) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if e.path().is_dir() {
                    out.push(name);
                }
            }
        }
        version::sort_desc(&mut out);
        out
    }

    pub fn is_installed(&self, tool: &str, version: &str) -> bool {
        self.paths.install_dir(tool, version).is_dir()
    }

    pub fn receipt(&self, tool: &str, version: &str) -> Option<Receipt> {
        installer::read_receipt(&self.paths, tool, version)
    }

    fn root_of(&self, tool: &str, version: &str) -> (PathBuf, Option<Receipt>) {
        let r = self.receipt(tool, version);
        let root = r
            .as_ref()
            .and_then(|r| r.home.clone())
            .unwrap_or_else(|| self.paths.install_dir(tool, version));
        (root, r)
    }

    pub fn bin_dirs(&self, tool: &str, version: &str) -> Vec<PathBuf> {
        let (root, r) = self.root_of(tool, version);
        if let Some(r) = r {
            if !r.bin_dirs.is_empty() {
                return r.bin_dirs;
            }
        }
        match self.registry.get(tool) {
            Some(p) => p.bin_dirs(&root),
            None => vec![root.join("bin")],
        }
    }

    pub fn env_vars(&self, tool: &str, version: &str) -> Vec<(String, String)> {
        let (root, _) = self.root_of(tool, version);
        self.registry
            .get(tool)
            .map(|p| p.env_vars(&root))
            .unwrap_or_default()
    }

    /// Match a spec against installed versions.
    pub fn match_installed(&self, tool: &str, spec: &str) -> Option<String> {
        let installed = self.installed(tool);
        if installed.iter().any(|v| v == spec) {
            return Some(spec.to_string());
        }
        VersionReq::parse(spec).best(installed.iter())
    }

    pub fn resolve(&self, tool: &str, cwd: &Path) -> Option<Resolved> {
        let env = |k: &str| std::env::var(k).ok();
        let spec = resolver::find_spec(tool, cwd, &env, &self.paths.global_tool_versions())?;
        Some(self.resolved_from_spec(spec))
    }

    fn resolved_from_spec(&self, spec: Spec) -> Resolved {
        let system = spec
            .specs
            .first()
            .map(|s| s.trim() == "system")
            .unwrap_or(false);
        let version = if system {
            None
        } else {
            spec.specs
                .iter()
                .find_map(|s| self.match_installed(&spec.tool, s))
        };
        Resolved {
            tool: spec.tool.clone(),
            spec,
            version,
            system,
        }
    }

    pub fn resolve_all(&self, cwd: &Path) -> Vec<Resolved> {
        self.registry
            .names()
            .iter()
            .filter_map(|t| self.resolve(t, cwd))
            .collect()
    }

    pub fn global_version(&self, tool: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.paths.global_tool_versions()).ok()?;
        toolversions::lookup(&text, tool)?.into_iter().next()
    }

    /// Resolve a fuzzy spec remotely and install it.
    pub fn install(
        &self,
        tool: &str,
        spec: &str,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<String> {
        let plugin = self.plugin(tool)?;
        let req = VersionReq::parse(spec);
        if req == VersionReq::System {
            bail!("system 表示使用系统自带版本，无需安装");
        }
        progress(Progress::Resolving {
            tool: plugin.name().into(),
            spec: spec.into(),
        });
        let net = self.net()?;
        let version = plugin
            .resolve_remote(&net, &req)
            .with_context(|| format!("解析 {tool} {spec} 失败"))?;
        installer::install(
            &self.paths,
            &net,
            plugin.as_ref(),
            &version,
            self.require_checksum,
            progress,
        )?;
        Ok(version)
    }

    /// Return an installed version matching `spec`, installing one if needed.
    pub fn ensure_installed(
        &self,
        tool: &str,
        spec: &str,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<String> {
        let tool = self.plugin(tool)?.name().to_string();
        if let Some(v) = self.match_installed(&tool, spec) {
            return Ok(v);
        }
        self.install(&tool, spec, progress)
    }

    pub fn uninstall(&self, tool: &str, version: &str) -> Result<()> {
        installer::validate_version_name(version)?;
        let tool = self.plugin(tool)?.name().to_string();
        let dir = self.paths.install_dir(&tool, version);
        if std::fs::symlink_metadata(&dir).is_err() {
            bail!("{tool} {version} 未安装");
        }
        // remove_link never follows symlinks, so adopted originals are left untouched
        platform::remove_link(&dir).with_context(|| format!("删除 {} 失败", dir.display()))?;
        let _ = std::fs::remove_file(installer::receipt_path(&self.paths, &tool, version));
        let cur = self.paths.current().join(&tool);
        if let Ok(target) = std::fs::read_link(&cur) {
            if target == dir {
                let _ = platform::remove_link(&cur);
            }
        }
        Ok(())
    }

    /// Write `tool version` to the global `.tool-versions` and update `current/<tool>`.
    pub fn set_global(&self, tool: &str, version: &str) -> Result<PathBuf> {
        let tool = self.plugin(tool)?.name().to_string();
        let file = self.paths.global_tool_versions();
        toolversions::set_in_file(&file, &tool, Some(version))?;
        self.update_current_link(&tool, version);
        Ok(file)
    }

    /// Write `tool version` to `<dir>/.tool-versions`.
    pub fn set_local(&self, dir: &Path, tool: &str, version: &str) -> Result<PathBuf> {
        let tool = self.plugin(tool)?.name().to_string();
        let file = dir.join(".tool-versions");
        toolversions::set_in_file(&file, &tool, Some(version))?;
        Ok(file)
    }

    pub fn unset(&self, file: &Path, tool: &str) -> Result<()> {
        toolversions::set_in_file(file, tool, None)
    }

    /// `~/.runa/current/<tool>` -> install dir (handy for IDE settings such as JAVA_HOME).
    pub fn update_current_link(&self, tool: &str, version: &str) {
        let link = self.paths.current().join(tool);
        let _ = std::fs::create_dir_all(self.paths.current());
        let _ = platform::remove_link(&link);
        let target = self.paths.install_dir(tool, version);
        if target.is_dir() {
            let _ = platform::symlink_dir(&target, &link);
        }
    }

    /// Environment for `cwd`. `overrides` are `(tool, spec)` pairs that win over files.
    pub fn env_plan(&self, cwd: &Path, overrides: &[(String, String)]) -> Result<EnvPlan> {
        let mut plan = EnvPlan::default();
        let mut tools: Vec<Resolved> = Vec::new();
        for (tool, spec) in overrides {
            let tool = self.plugin(tool)?.name().to_string();
            let s = Spec {
                tool: tool.clone(),
                specs: vec![spec.clone()],
                source: resolver::Source::Env("--with".into()),
            };
            let r = self.resolved_from_spec(s);
            if r.version.is_none() && !r.system {
                bail!("{tool} {spec} 未安装，请先运行: runa install {tool} {spec}");
            }
            tools.push(r);
        }
        for r in self.resolve_all(cwd) {
            if !tools.iter().any(|t| t.tool == r.tool) {
                tools.push(r);
            }
        }
        for r in &tools {
            if let Some(v) = &r.version {
                for d in self.bin_dirs(&r.tool, v) {
                    if !plan.paths.contains(&d) {
                        plan.paths.push(d);
                    }
                }
                plan.vars.extend(self.env_vars(&r.tool, v));
            }
        }
        if let Some(p) = &self.project {
            plan.vars.extend(p.variables());
            for k in p.cleared_variables() {
                plan.vars.push((k.to_string(), String::new()));
            }
            if p.mode == "local" {
                let bin = p
                    .env_root()
                    .join("node")
                    .join(if cfg!(windows) { "" } else { "bin" });
                let gobin = p.env_root().join("go").join("bin");
                for d in [bin, gobin] {
                    if !plan.paths.contains(&d) {
                        plan.paths.push(d);
                    }
                }
            }
        }
        plan.tools = tools;
        Ok(plan)
    }

    /// Path of `exe` as it would run in `cwd`.
    pub fn which(&self, exe: &str, cwd: &Path) -> Result<Option<PathBuf>> {
        let plan = self.env_plan(cwd, &[])?;
        if let Some(p) = find_exe_in(&plan.paths, exe) {
            return Ok(Some(p));
        }
        Ok(find_exe_in(&self.system_path(), exe))
    }

    /// PATH entries of this process minus runa's own shims and installs.
    pub fn system_path(&self) -> Vec<PathBuf> {
        let base = std::env::var_os("RUNA_PATH_BASE")
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        std::env::split_paths(&base)
            .filter(|p| !p.as_os_str().is_empty() && !self.paths.contains(p))
            .collect()
    }

    /// Executables provided by every installed version (shim names).
    pub fn all_commands(&self) -> Vec<String> {
        let mut names = std::collections::BTreeSet::new();
        for tool in self.registry.names() {
            for v in self.installed(&tool) {
                for d in self.bin_dirs(&tool, &v) {
                    let Ok(rd) = std::fs::read_dir(&d) else {
                        continue;
                    };
                    for e in rd.flatten() {
                        let p = e.path();
                        if platform::is_executable(&p) {
                            if let Some(n) = platform::command_stem(&p) {
                                if !n.starts_with('.') {
                                    names.insert(n);
                                }
                            }
                        }
                    }
                }
            }
        }
        names.into_iter().collect()
    }

    /// Which installed tools provide `exe` (for helpful error messages).
    pub fn providers_of(&self, exe: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for tool in self.registry.names() {
            for v in self.installed(&tool) {
                if find_exe_in(&self.bin_dirs(&tool, &v), exe).is_some() {
                    out.push((tool.clone(), v));
                }
            }
        }
        out
    }

    pub fn status(&self, cwd: &Path) -> Vec<ToolStatus> {
        self.registry
            .all()
            .iter()
            .map(|p| {
                let tool = p.name().to_string();
                let installed = self
                    .installed(&tool)
                    .into_iter()
                    .map(|v| {
                        let r = self.receipt(&tool, &v);
                        InstalledVersion {
                            path: self.paths.install_dir(&tool, &v),
                            source: r
                                .as_ref()
                                .map(|r| r.source.clone())
                                .unwrap_or_else(|| "download".into()),
                            origin: r.and_then(|r| r.origin),
                            version: v,
                        }
                    })
                    .collect();
                ToolStatus {
                    description: p.description(),
                    builtin: p.builtin(),
                    global: self.global_version(&tool),
                    current: self.resolve(&tool, cwd),
                    installed,
                    tool,
                }
            })
            .collect()
    }
}

/// Bytes used by a directory tree. Symlinks are not followed, so adopted runtimes count ~0.
pub fn dir_size(p: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if meta.is_file() {
        return meta.len();
    }
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            total += dir_size(&e.path());
        }
    }
    total
}

/// Look for `exe` in `dirs` (adds .exe/.cmd/.bat on Windows).
pub fn find_exe_in(dirs: &[PathBuf], exe: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) && Path::new(exe).extension().is_none() {
        ["exe", "cmd", "bat", "com"]
            .iter()
            .map(|e| format!("{exe}.{e}"))
            .collect()
    } else {
        vec![exe.to_string()]
    };
    for d in dirs {
        for n in &names {
            let p = d.join(n);
            if platform::is_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}
