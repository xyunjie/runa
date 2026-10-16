//! `runa context`: read-only version context for agents / AI tools.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use runa_core::manager::find_exe_in;
use runa_core::project;
use runa_core::version::VersionReq;
use runa_core::Manager;
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Suggestion {
    pub command: String,
    pub args: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct ToolContext {
    pub tool: String,
    pub requested_version: String,
    pub resolved_version: String,
    pub active_version: String,
    pub source: String,
    pub project_requirement: String,
    pub project_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    pub state: String,
    pub override_conflict: bool,
    pub suggestions: Vec<Suggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn sug(args: &[&str], reason: &str) -> Suggestion {
    Suggestion {
        command: "runa".into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        reason: reason.into(),
    }
}

/// Inputs for one tool, kept separate from the filesystem so the state machine is testable.
pub struct Facts<'a> {
    pub tool: &'a str,
    pub requested: &'a str,
    pub resolved: Option<&'a str>,
    pub system: bool,
    pub project_requirement: Option<&'a str>,
    pub docker: bool,
    /// The command found on PATH runs the resolved version (directly or via a runa shim).
    pub active: bool,
}

/// Decide state + suggestions (mirrors Vaelen's `vsm context`).
pub fn evaluate(f: &Facts) -> (String, bool, Vec<Suggestion>) {
    let mut s = vec![];
    let mut state = "ready";
    if f.docker {
        state = "container_unverified";
        s.push(sug(
            &["run", "--", f.tool, "<args>"],
            "在项目 Docker 镜像中验证版本并执行",
        ));
    } else if f.system {
        state = "ready";
    } else if f.resolved.is_none() {
        state = "missing_version";
        s.push(sug(
            &["install", f.tool, f.requested, "--require-checksum"],
            "安装项目要求的版本，并要求发布方校验值",
        ));
    } else if !f.active {
        state = "activation_required";
        s.push(sug(
            &["run", "--", f.tool, "<args>"],
            "自动选择项目声明的所有工具版本并执行，无需修改父 shell",
        ));
        s.push(sug(
            &["shell", f.tool, f.requested],
            "在临时子 shell 中使用该版本，退出后恢复",
        ));
        s.push(sug(
            &["exec", "--", f.tool, "<args>"],
            "让 Agent 命令明确使用解析出的版本",
        ));
    }
    let mut conflict = false;
    if let (Some(req), Some(v)) = (f.project_requirement, f.resolved) {
        if !f.docker && !VersionReq::parse(req).matches(v) && req != v {
            conflict = true;
            state = "override_conflict";
            s.push(sug(
                &["shell", f.tool, req],
                "当前环境覆盖与项目要求不同，显式选择项目版本",
            ));
            s.push(sug(
                &["run", "--", f.tool, "<args>"],
                "按项目声明执行，覆盖继承的旧版本设置",
            ));
        }
    }
    (state.into(), conflict, s)
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn collect(m: &Manager, cwd: &Path) -> Vec<ToolContext> {
    let docker = m
        .project
        .as_ref()
        .map(|p| p.mode == "docker")
        .unwrap_or(false);
    let path: Vec<PathBuf> =
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    let global_shims = runa_core::Paths::with_root(m.paths.home.clone()).shims();
    let mut out = vec![];
    for tool in m.registry.names() {
        let r = m.resolve(&tool, cwd);
        let req = project::requirement(&tool, cwd);
        if r.is_none() && req.is_none() {
            continue;
        }
        let requested = r
            .as_ref()
            .map(|r| r.wanted().to_string())
            .unwrap_or_default();
        let resolved = r.as_ref().and_then(|r| r.version.clone());
        let system = r.as_ref().map(|r| r.system).unwrap_or(false);
        let main = m
            .registry
            .get(&tool)
            .and_then(|p| p.detect_exes().first().map(|s| s.to_string()))
            .unwrap_or_else(|| tool.clone());
        let actual = find_exe_in(&path, &main);
        let mut active = false;
        if let (Some(v), Some(a)) = (&resolved, &actual) {
            let via_shim = a
                .parent()
                .map(|d| canon(d) == canon(&m.paths.shims()) || canon(d) == canon(&global_shims))
                .unwrap_or(false);
            let expected = find_exe_in(&m.bin_dirs(&tool, v), &main);
            active = via_shim || expected.map(|e| canon(&e) == canon(a)).unwrap_or(false);
        }
        let req_spec = req.as_ref().and_then(|s| s.specs.first().cloned());
        let (state, conflict, suggestions) = evaluate(&Facts {
            tool: &tool,
            requested: &requested,
            resolved: resolved.as_deref(),
            system,
            project_requirement: req_spec.as_deref(),
            docker,
            active,
        });
        out.push(ToolContext {
            requested_version: requested,
            resolved_version: if system {
                "system".into()
            } else {
                resolved.clone().unwrap_or_default()
            },
            active_version: if active {
                resolved.clone().unwrap_or_default()
            } else {
                String::new()
            },
            source: r
                .as_ref()
                .map(|r| r.spec.source.describe())
                .unwrap_or_default(),
            project_requirement: req_spec.unwrap_or_default(),
            project_source: req.map(|s| s.source.describe()).unwrap_or_default(),
            executable: actual,
            state,
            override_conflict: conflict,
            suggestions,
            error: None,
            tool,
        });
    }
    out
}

pub fn command(m: &Manager, cwd: &Path, json: bool, check: bool) -> Result<i32> {
    let items = collect(m, cwd);
    let ready = items.iter().all(|i| i.state == "ready");
    let status = if ready { "ready" } else { "attention_required" };
    if json {
        let v = serde_json::json!({
            "schema_version": 1,
            "runa_version": env!("CARGO_PKG_VERSION"),
            "directory": cwd,
            "status": status,
            "tools": items,
            "read_only": true,
            "project": m.project,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("项目: {}\n状态: {status}", cwd.display());
        for i in &items {
            println!(
                "{}: 项目要求={} 解析={} 已激活={} [{}]",
                i.tool, i.project_requirement, i.resolved_version, i.active_version, i.state
            );
            for s in &i.suggestions {
                println!("  {} {} — {}", s.command, s.args.join(" "), s.reason);
            }
        }
    }
    if check && !ready {
        bail!("版本环境尚未满足项目要求，参见 context 输出");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>() -> Facts<'a> {
        Facts {
            tool: "node",
            requested: "20",
            resolved: Some("20.11.1"),
            system: false,
            project_requirement: Some("20"),
            docker: false,
            active: true,
        }
    }

    #[test]
    fn ready_when_active() {
        let (s, c, sug) = evaluate(&facts());
        assert_eq!(s, "ready");
        assert!(!c && sug.is_empty());
    }

    #[test]
    fn missing_and_activation() {
        let mut f = facts();
        f.resolved = None;
        let (s, _, sug) = evaluate(&f);
        assert_eq!(s, "missing_version");
        assert!(sug[0].args.contains(&"--require-checksum".to_string()));
        let mut f = facts();
        f.active = false;
        assert_eq!(evaluate(&f).0, "activation_required");
    }

    #[test]
    fn override_conflict_and_docker() {
        let mut f = facts();
        f.resolved = Some("18.19.0");
        f.requested = "18";
        let (s, c, _) = evaluate(&f);
        assert_eq!(s, "override_conflict");
        assert!(c);
        let mut f = facts();
        f.docker = true;
        assert_eq!(evaluate(&f).0, "container_unverified");
    }
}
