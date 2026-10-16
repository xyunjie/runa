//! `runa mcp`: the global MCP server (stdio, built on `rmcp`).
//!
//! Tools reuse runa-core directly. Write tools obey [`runa_core::mcp::Policy`] from
//! `settings.json` (re-read on every call); operations that need confirmation are sent to
//! the running desktop GUI over the localhost bridge, and the policy is checked again after
//! approval. stdout carries only MCP messages: progress and logs go to stderr.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use runa_core::mcp::{self as core_mcp, ApprovalRequest, OpKind, Operation, Policy};
use runa_core::uninstall::{self, Mode, PlanKind};
use runa_core::{activate, managers, Manager, Paths, Settings};
use serde_json::{json, Map, Value};

// ---------------------------------------------------------------- tools

fn tool(name: &str, title: &str, desc: &str, props: Value, required: &[&str], ann: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": { "type": "object", "properties": props, "required": required, "additionalProperties": false },
        "annotations": ann,
    })
}

fn read_ann(title: &str, open_world: bool) -> Value {
    json!({ "title": title, "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": open_world })
}

fn write_ann(title: &str, destructive: bool, idempotent: bool) -> Value {
    json!({ "title": title, "readOnlyHint": false, "destructiveHint": destructive, "idempotentHint": idempotent, "openWorldHint": !destructive })
}

/// Tool definitions (JSON, as sent in `tools/list`).
pub fn tools() -> Vec<Value> {
    let s = |d: &str| json!({ "type": "string", "description": d });
    let rt = || {
        s("Runtime name: node, java, python, go, rust, ruby, bun, deno (or a registered plugin)")
    };
    vec![
        tool("list_runtimes", "List installed runtimes",
            "List installed versions of every runtime managed by runa and the current global version of each. Read-only.",
            json!({ "runtime": rt() }), &[], read_ann("List installed runtimes", false)),
        tool("list_available", "List installable versions",
            "List versions of a runtime that can be installed (newest first). Read-only; queries the publisher's index over the network.",
            json!({ "runtime": rt(), "include_prerelease": { "type": "boolean", "default": false }, "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 } }),
            &["runtime"], read_ann("List installable versions", true)),
        tool("detect_project", "Detect project versions",
            "Read which runtime versions a project directory requires (.tool-versions, .nvmrc, .node-version, .python-version, .java-version, .ruby-version, go.mod, rust-toolchain, ...) and whether each is installed. Read-only.",
            json!({ "dir": s("Absolute path of the project directory") }), &["dir"], read_ann("Detect project versions", false)),
        tool("list_managers", "List version managers",
            "List version managers runa knows on this platform (nvm, pyenv, sdkman, mise, asdf, rustup, rbenv, brew, winget, scoop, ...) with install state and the runtimes registered through them. Read-only.",
            json!({}), &[], read_ann("List version managers", false)),
        tool("install", "Install a runtime version",
            "Install a runtime version (exact version, prefix like 20 or 3.12, or latest), through runa's verified download or a version manager (via). If the runtime has no global version yet, it becomes the global one. Requires MCP write mode in Runa; may require approval in the Runa desktop app.",
            json!({ "runtime": rt(), "version": s("Version, numeric prefix, lts or latest"), "via": s("Optional installer: runa, or a manager name such as nvm, pyenv, sdkman, mise, asdf, rustup, rbenv, brew") }),
            &["runtime", "version"], write_ann("Install a runtime version", false, true)),
        tool("set_global", "Set the global version",
            "Make an installed version the global default (~/.runa/.tool-versions). Requires MCP write mode in Runa; may require approval in the Runa desktop app.",
            json!({ "runtime": rt(), "version": s("Installed version (exact, or a prefix matching an installed version)") }),
            &["runtime", "version"], write_ann("Set the global version", false, true)),
        tool("uninstall", "Uninstall a runtime version",
            "Uninstall an installed version. mode auto (default) deletes runa downloads; for runtimes adopted from another manager use forget (stop managing, keep files) or purge (remove through the owning tool). System-bundled runtimes can never be uninstalled. Requires MCP write mode and, by default, approval in the Runa desktop app.",
            json!({ "runtime": rt(), "version": s("Exact installed version"), "mode": { "type": "string", "enum": ["auto", "forget", "purge"], "default": "auto" } }),
            &["runtime", "version"], write_ann("Uninstall a runtime version", true, false)),
    ]
}

// ---------------------------------------------------------------- context

/// Asks a human to approve one exact operation.
pub trait Approver: Send + Sync {
    fn approve(&self, home: &Path, req: &ApprovalRequest) -> Result<()>;
}

/// The real approver: the desktop GUI over the localhost bridge.
pub struct GuiApprover;
impl Approver for GuiApprover {
    fn approve(&self, home: &Path, req: &ApprovalRequest) -> Result<()> {
        eprintln!("runa mcp: 等待在 Runa 桌面端确认：{}", req.summary);
        core_mcp::request_approval(home, req, Duration::from_secs(300))
    }
}

pub struct Ctx {
    /// `None`: the user's runa home (`RUNA_HOME` / `~/.runa`).
    pub paths: Option<Paths>,
    pub approver: Arc<dyn Approver>,
    pub client: Mutex<Option<String>>,
    pub cli: PathBuf,
}

impl Ctx {
    fn manager(&self) -> Result<Manager> {
        match &self.paths {
            Some(p) => Manager::with_paths(p.clone()),
            None => Manager::load(),
        }
    }

    fn policy(&self, m: &Manager) -> Result<Policy> {
        // re-read from disk: the GUI may have changed it while we were waiting
        Ok(Settings::load(&m.paths.settings_file())?.mcp)
    }

    /// Policy check, approval when required, then a second policy check.
    fn authorize(
        &self,
        m: &Manager,
        op: Operation,
        summary: String,
        details: Vec<String>,
    ) -> Result<bool> {
        let kind = op.kind;
        let p = self.policy(m)?;
        p.check(kind)?;
        if !p.needs_approval(kind) {
            return Ok(false);
        }
        let client = self.client.lock().unwrap().clone();
        let req = ApprovalRequest::new(op, summary, details, client);
        self.approver.approve(&m.paths.home, &req)?;
        self.policy(m)?.check(kind)?;
        Ok(true)
    }
}

// ---------------------------------------------------------------- handlers

fn args_for<'a>(name: &str, args: &'a Map<String, Value>) -> Result<Args<'a>> {
    let defs = tools();
    let def = defs
        .iter()
        .find(|t| t["name"] == name)
        .ok_or_else(|| anyhow!("未知工具 {name}"))?;
    let props = def["inputSchema"]["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for k in args.keys() {
        if !props.contains_key(k) {
            bail!("{name} 不接受参数 {k:?}");
        }
    }
    for r in def["inputSchema"]["required"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let r = r.as_str().unwrap_or_default();
        if !args.contains_key(r) {
            bail!("{name} 缺少参数 {r}");
        }
    }
    Ok(Args(args))
}

struct Args<'a>(&'a Map<String, Value>);
impl Args<'_> {
    fn str(&self, k: &str) -> Result<Option<String>> {
        match self.0.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(Value::String(s)) => {
                if s.len() > 200 || s.chars().any(|c| c.is_control()) {
                    bail!("参数 {k} 无效");
                }
                Ok(Some(s.trim().to_string()))
            }
            Some(_) => bail!("参数 {k} 必须是字符串"),
        }
    }
    fn req(&self, k: &str) -> Result<String> {
        self.str(k)?.ok_or_else(|| anyhow!("缺少参数 {k}"))
    }
    fn bool(&self, k: &str) -> Result<bool> {
        match self.0.get(k) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => bail!("参数 {k} 必须是布尔值"),
        }
    }
    fn int(&self, k: &str, default: u64, max: u64) -> Result<u64> {
        match self.0.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_u64()
                .filter(|n| (1..=max).contains(n))
                .ok_or_else(|| anyhow!("参数 {k} 必须是 1..{max} 的整数")),
        }
    }
}

fn runtime(m: &Manager, a: &Args) -> Result<String> {
    Ok(m.plugin(&a.req("runtime")?)?.name().to_string())
}

/// Run one tool. Errors become `isError` tool results.
pub fn call(ctx: &Ctx, name: &str, raw: &Map<String, Value>) -> Result<Value> {
    let a = args_for(name, raw)?;
    let m = ctx.manager()?;
    match name {
        "list_runtimes" => {
            let only = match a.str("runtime")? {
                Some(_) => Some(runtime(&m, &a)?),
                None => None,
            };
            let home = m.paths.home.clone();
            let rts: Vec<Value> = m
                .status(&home)
                .into_iter()
                .filter(|t| only.as_ref().is_none_or(|o| o == &t.tool))
                .map(|t| {
                    json!({
                        "runtime": t.tool,
                        "global": t.global,
                        "installed": t.installed.iter().map(|v| json!({
                            "version": v.version,
                            "source": v.source,
                            "provider": v.provider,
                            "disabled": v.disabled,
                            "path": v.path,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({ "runtimes": rts }))
        }
        "list_available" => {
            let t = runtime(&m, &a)?;
            let pre = a.bool("include_prerelease")?;
            let limit = a.int("limit", 50, 500)? as usize;
            let all = m.plugin(&t)?.list_remote(&m.net()?)?;
            let total = all.len();
            let versions: Vec<Value> = all
                .into_iter()
                .filter(|v| pre || !v.prerelease)
                .take(limit)
                .map(|v| {
                    let installed = m.is_installed(&t, &v.version);
                    json!({ "version": v.version, "lts": v.lts, "prerelease": v.prerelease, "installed": installed })
                })
                .collect();
            Ok(json!({ "runtime": t, "total": total, "versions": versions }))
        }
        "detect_project" => {
            let dir = PathBuf::from(a.req("dir")?);
            if !dir.is_absolute() {
                bail!("dir 必须是绝对路径");
            }
            if !dir.is_dir() {
                bail!("目录不存在: {}", dir.display());
            }
            let req: Vec<Value> = m
                .resolve_all(&dir)
                .into_iter()
                .filter(|r| !matches!(r.spec.source, runa_core::Source::Global(_)))
                .map(|r| {
                    let file = match &r.spec.source {
                        runa_core::Source::File(p) => Some(p.display().to_string()),
                        _ => None,
                    };
                    json!({
                        "runtime": r.tool,
                        "required": r.spec.specs,
                        "source": r.spec.source.describe(),
                        "file": file,
                        "system": r.system,
                        "installed_version": r.version,
                        "satisfied": r.system || r.version.is_some(),
                    })
                })
                .collect();
            Ok(json!({ "dir": dir, "requirements": req }))
        }
        "list_managers" => {
            let probe = managers::Probe::current(&m);
            let v: Vec<Value> = runa_core::manager_admin::info(&m, &probe)
                .into_iter()
                .map(|i| {
                    json!({
                        "name": i.name,
                        "runtimes_supported": i.tools,
                        "installed": i.installed,
                        "version": i.version,
                        "origin": i.origin,
                        "registered_runtimes": i.runtimes.iter().map(|r| format!("{} {}", r.tool, r.version)).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let prefs = &m.settings.install_via;
            Ok(json!({ "managers": v, "install_via": prefs }))
        }
        "install" => {
            ctx.policy(&m)?.check(OpKind::Install)?;
            let t = runtime(&m, &a)?;
            let spec = a.req("version")?;
            if spec.starts_with('-') || spec.contains(['/', '\\']) {
                bail!("version 无效");
            }
            let via = a.str("via")?;
            let op = Operation {
                kind: OpKind::Install,
                tool: t.clone(),
                version: spec.clone(),
                via: via.clone(),
                mode: None,
            };
            let how = via
                .as_deref()
                .map(|v| format!("（通过 {v}）"))
                .unwrap_or_default();
            let approved = ctx.authorize(&m, op, format!("安装 {t} {spec}{how}"), vec![])?;
            let mut last = String::new();
            let (v, used) = managers::install(&m, &t, &spec, via.as_deref(), &mut |p| {
                let line = format!("{p:?}");
                if line != last {
                    eprintln!("runa mcp: {line}");
                    last = line;
                }
            })?;
            let set_global = m.global_version(&t).is_none();
            if set_global {
                m.set_global(&t, &v)?;
            }
            let _ = activate::reshim(&m, &ctx.cli);
            Ok(
                json!({ "runtime": t, "version": v, "via": used, "set_global": set_global, "approved_in_gui": approved }),
            )
        }
        "set_global" => {
            ctx.policy(&m)?.check(OpKind::SetGlobal)?;
            let t = runtime(&m, &a)?;
            let spec = a.req("version")?;
            let v = m
                .match_installed(&t, &spec)
                .ok_or_else(|| anyhow!("{t} {spec} 未安装（或已禁用），请先安装"))?;
            let before = m.global_version(&t);
            let op = Operation {
                kind: OpKind::SetGlobal,
                tool: t.clone(),
                version: v.clone(),
                via: None,
                mode: None,
            };
            let approved = ctx.authorize(
                &m,
                op,
                format!("把 {t} 的全局版本设为 {v}"),
                vec![format!(
                    "当前全局版本：{}",
                    before.as_deref().unwrap_or("无")
                )],
            )?;
            let file = m.set_global(&t, &v)?;
            let _ = activate::reshim(&m, &ctx.cli);
            Ok(
                json!({ "runtime": t, "version": v, "previous": before, "file": file, "approved_in_gui": approved }),
            )
        }
        "uninstall" => {
            ctx.policy(&m)?.check(OpKind::Uninstall)?;
            let t = runtime(&m, &a)?;
            let version = a.req("version")?;
            let mode_s = a.str("mode")?.unwrap_or_else(|| "auto".into());
            let mode = match mode_s.as_str() {
                "auto" => Mode::Auto,
                "forget" => Mode::Forget,
                "purge" => Mode::Purge,
                _ => bail!("mode 必须是 auto / forget / purge"),
            };
            // core refuses system runtimes (hard error) before anything is asked
            let plan = uninstall::plan(&m, &t, &version, mode)?;
            if plan.kind == PlanKind::Refuse {
                bail!("无法卸载：{}", plan.summary);
            }
            if plan.needs_root {
                bail!(
                    "{} 需要管理员权限，runa 不会代为执行；请手动运行：{}",
                    plan.summary,
                    plan.command_line.clone().unwrap_or_default()
                );
            }
            let mut details: Vec<String> = plan
                .paths
                .iter()
                .map(|p| format!("删除 {}", p.display()))
                .collect();
            if let Some(c) = &plan.command_line {
                details.insert(0, format!("执行 {c}"));
            }
            let op = Operation {
                kind: OpKind::Uninstall,
                tool: t.clone(),
                version: version.clone(),
                via: plan.provider.clone(),
                mode: Some(mode_s.clone()),
            };
            let approved = ctx.authorize(
                &m,
                op,
                format!("卸载 {t} {version}：{}", plan.summary),
                details,
            )?;
            // re-plan: execute only what was shown
            let again = uninstall::plan(&m, &t, &version, mode)?;
            if again.kind != plan.kind || again.paths != plan.paths || again.command != plan.command
            {
                bail!("确认期间卸载计划发生了变化，未执行，请重试");
            }
            uninstall::execute(&m, &again)?;
            let _ = activate::reshim(&m, &ctx.cli);
            Ok(
                json!({ "runtime": t, "version": version, "mode": mode_s, "summary": plan.summary, "approved_in_gui": approved }),
            )
        }
        _ => bail!("未知工具 {name}"),
    }
}

// ---------------------------------------------------------------- rmcp glue

#[derive(Clone)]
struct RunaServer {
    ctx: Arc<Ctx>,
}

impl ServerHandler for RunaServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("runa", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Runa manages language runtimes (node, java, python, go, rust, ruby, bun, deno). \
                 Use list_runtimes / detect_project / list_available before changing anything. \
                 Write tools follow the policy the user set in the Runa desktop app (read-only by default); \
                 uninstalls are confirmed by the user in the Runa app.",
            )
    }

    async fn list_tools(
        &self,
        _r: Option<PaginatedRequestParams>,
        _c: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let ts: Vec<Tool> = tools()
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(ts))
    }

    async fn call_tool(
        &self,
        r: CallToolRequestParams,
        c: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !tools().iter().any(|t| t["name"] == *r.name) {
            return Err(ErrorData::invalid_params(
                format!("unknown tool {}", r.name),
                None,
            ));
        }
        if let Some(info) = c.peer.peer_info() {
            *self.ctx.client.lock().unwrap() = Some(info.client_info.name.clone());
        }
        let ctx = self.ctx.clone();
        let name = r.name.to_string();
        let args = r.arguments.unwrap_or_default();
        let res = tokio::task::spawn_blocking(move || call(&ctx, &name, &args))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(match res {
            Ok(v) => CallToolResult::structured(v),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))]),
        }
        .into())
    }
}

pub fn run() -> Result<i32> {
    let ctx = Arc::new(Ctx {
        paths: None,
        approver: Arc::new(GuiApprover),
        client: Mutex::new(None),
        cli: std::env::current_exe().unwrap_or_else(|_| activate::cli_path()),
    });
    // fail early (on stderr) if the runa home is unusable
    ctx.manager()?;
    eprintln!("runa mcp: MCP 服务已启动（stdio）");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let svc = RunaServer { ctx }
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| anyhow!("MCP 初始化失败: {e}"))?;
        svc.waiting().await?;
        anyhow::Ok(())
    })?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use runa_core::installer;

    struct Fake {
        answer: bool,
        seen: Mutex<Vec<ApprovalRequest>>,
        /// Flip the policy back to read-only while "waiting" for the user.
        revoke: Option<PathBuf>,
    }
    impl Approver for Fake {
        fn approve(&self, _home: &Path, req: &ApprovalRequest) -> Result<()> {
            self.seen.lock().unwrap().push(req.clone());
            if let Some(f) = &self.revoke {
                let mut s = Settings::load(f)?;
                s.mcp.mode = "read_only".into();
                s.save(f)?;
            }
            if self.answer {
                Ok(())
            } else {
                bail!("用户拒绝")
            }
        }
    }

    fn setup(answer: bool, revoke: bool) -> (tempfile::TempDir, Ctx, Arc<Fake>) {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(t.path());
        paths.ensure().unwrap();
        let fake = Arc::new(Fake {
            answer,
            seen: Mutex::new(vec![]),
            revoke: revoke.then(|| paths.settings_file()),
        });
        let ctx = Ctx {
            approver: fake.clone(),
            paths: Some(paths),
            client: Mutex::new(Some("test-client".into())),
            cli: PathBuf::from("/nonexistent/runa"),
        };
        (t, ctx, fake)
    }

    fn set_policy(ctx: &Ctx, p: Policy) {
        let f = ctx.paths.as_ref().unwrap().settings_file();
        let mut s = Settings::load(&f).unwrap();
        s.mcp = p;
        s.save(&f).unwrap();
    }

    /// A fake runa-downloaded install (dir + receipt).
    fn fake_install(ctx: &Ctx, tool: &str, v: &str) {
        let m = ctx.manager().unwrap();
        let dir = m.paths.install_dir(tool, v);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        installer::write_receipt(
            &m.paths,
            &installer::Receipt {
                tool: tool.into(),
                version: v.into(),
                source: "download".into(),
                ..Default::default()
            },
        )
        .unwrap();
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    fn seen(f: &Fake) -> usize {
        f.seen.lock().unwrap().len()
    }

    #[test]
    fn tool_list_and_annotations() {
        let ts = tools();
        let names: Vec<&str> = ts.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "list_runtimes",
                "list_available",
                "detect_project",
                "list_managers",
                "install",
                "set_global",
                "uninstall"
            ]
        );
        for t in &ts {
            let ro = t["annotations"]["readOnlyHint"].as_bool().unwrap();
            assert_eq!(
                ro,
                [
                    "list_runtimes",
                    "list_available",
                    "detect_project",
                    "list_managers"
                ]
                .contains(&t["name"].as_str().unwrap())
            );
            assert_eq!(
                t["annotations"]["destructiveHint"].as_bool().unwrap(),
                t["name"] == "uninstall"
            );
            // valid rmcp tools
            serde_json::from_value::<Tool>(t.clone()).unwrap();
        }
    }

    #[test]
    fn read_tools() {
        let (t, ctx, _f) = setup(true, false);
        fake_install(&ctx, "node", "20.11.1");
        let r = call(&ctx, "list_runtimes", &obj(json!({}))).unwrap();
        let node = r["runtimes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["runtime"] == "node")
            .unwrap()
            .clone();
        assert_eq!(node["installed"][0]["version"], "20.11.1");
        let r = call(&ctx, "list_runtimes", &obj(json!({"runtime": "node"}))).unwrap();
        assert_eq!(r["runtimes"].as_array().unwrap().len(), 1);
        assert!(call(&ctx, "list_runtimes", &obj(json!({"runtime": "cobol"}))).is_err());
        assert!(call(&ctx, "list_runtimes", &obj(json!({"bogus": 1}))).is_err());
        // project detection reuses the resolver (.nvmrc, .python-version, .tool-versions)
        let proj = t.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(".nvmrc"), "20\n").unwrap();
        std::fs::write(proj.join(".python-version"), "3.12\n").unwrap();
        let r = call(&ctx, "detect_project", &obj(json!({"dir": proj}))).unwrap();
        let reqs = r["requirements"].as_array().unwrap();
        let node = reqs.iter().find(|x| x["runtime"] == "node").unwrap();
        assert_eq!(node["required"], json!(["20"]));
        assert_eq!(node["installed_version"], "20.11.1");
        assert_eq!(node["satisfied"], true);
        let py = reqs.iter().find(|x| x["runtime"] == "python").unwrap();
        assert_eq!(py["satisfied"], false);
        assert!(call(&ctx, "detect_project", &obj(json!({"dir": "relative"}))).is_err());
        assert!(call(&ctx, "detect_project", &obj(json!({}))).is_err());
        assert!(call(&ctx, "list_managers", &obj(json!({}))).unwrap()["managers"].is_array());
    }

    #[test]
    fn writes_refused_in_read_only_mode() {
        let (_t, ctx, fake) = setup(true, false);
        fake_install(&ctx, "node", "20.11.1");
        for (name, args) in [
            ("install", json!({"runtime": "node", "version": "22"})),
            (
                "set_global",
                json!({"runtime": "node", "version": "20.11.1"}),
            ),
            (
                "uninstall",
                json!({"runtime": "node", "version": "20.11.1"}),
            ),
        ] {
            let e = call(&ctx, name, &obj(args)).unwrap_err().to_string();
            assert!(e.contains("只读模式"), "{name}: {e}");
        }
        assert_eq!(seen(&fake), 0);
        assert!(ctx.manager().unwrap().is_installed("node", "20.11.1"));
    }

    #[test]
    fn set_global_and_uninstall_follow_policy() {
        let (_t, ctx, fake) = setup(true, false);
        fake_install(&ctx, "node", "20.11.1");
        fake_install(&ctx, "node", "22.1.0");
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                ..Default::default()
            },
        );
        // set_global: no approval by default; must be installed
        assert!(call(
            &ctx,
            "set_global",
            &obj(json!({"runtime": "node", "version": "18"}))
        )
        .is_err());
        let r = call(
            &ctx,
            "set_global",
            &obj(json!({"runtime": "node", "version": "22"})),
        )
        .unwrap();
        assert_eq!(r["version"], "22.1.0");
        assert_eq!(r["approved_in_gui"], false);
        assert_eq!(
            ctx.manager().unwrap().global_version("node").as_deref(),
            Some("22.1.0")
        );
        // uninstall: approval with the exact operation
        let r = call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "20.11.1"})),
        )
        .unwrap();
        assert_eq!(r["approved_in_gui"], true);
        assert!(!ctx.manager().unwrap().is_installed("node", "20.11.1"));
        let req = fake.seen.lock().unwrap()[0].clone();
        assert_eq!(req.op.kind, OpKind::Uninstall);
        assert_eq!(
            (req.op.tool.as_str(), req.op.version.as_str()),
            ("node", "20.11.1")
        );
        assert_eq!(req.client.as_deref(), Some("test-client"));
        assert!(!req.details.is_empty());
        // confirm_writes: set_global asks too
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                confirm_writes: true,
                ..Default::default()
            },
        );
        let r = call(
            &ctx,
            "set_global",
            &obj(json!({"runtime": "node", "version": "22.1.0"})),
        )
        .unwrap();
        assert_eq!(r["approved_in_gui"], true);
        assert_eq!(seen(&fake), 2);
        // confirm_destructive off: no approval
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                confirm_destructive: false,
                confirm_writes: false,
            },
        );
        fake_install(&ctx, "node", "20.11.1");
        let r = call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "20.11.1"})),
        )
        .unwrap();
        assert_eq!(r["approved_in_gui"], false);
        assert!(call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "9.9.9"}))
        )
        .is_err());
        assert!(call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "22.1.0", "mode": "nuke"}))
        )
        .is_err());
    }

    #[test]
    fn rejection_and_policy_recheck() {
        let (_t, ctx, _f) = setup(false, false);
        fake_install(&ctx, "node", "20.11.1");
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                ..Default::default()
            },
        );
        let e = call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "20.11.1"})),
        )
        .unwrap_err();
        assert!(e.to_string().contains("用户拒绝"));
        assert!(ctx.manager().unwrap().is_installed("node", "20.11.1"));
        // approved, but the user switched to read-only meanwhile
        let (_t2, ctx, _f2) = setup(true, true);
        fake_install(&ctx, "node", "20.11.1");
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                ..Default::default()
            },
        );
        let e = call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "20.11.1"})),
        )
        .unwrap_err();
        assert!(e.to_string().contains("只读模式"), "{e}");
        assert!(ctx.manager().unwrap().is_installed("node", "20.11.1"));
    }

    #[test]
    fn system_runtimes_are_never_uninstallable() {
        let (_t, ctx, fake) = setup(true, false);
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                ..Default::default()
            },
        );
        let m = ctx.manager().unwrap();
        installer::write_receipt(
            &m.paths,
            &installer::Receipt {
                tool: "python".into(),
                version: "3.9.6".into(),
                source: "adopted".into(),
                home: Some("/usr".into()),
                origin: Some("/usr".into()),
                bin_dirs: vec![PathBuf::from("/usr/bin")],
                provider: Some("system".into()),
                ..Default::default()
            },
        )
        .unwrap();
        for mode in ["auto", "purge"] {
            let e = call(
                &ctx,
                "uninstall",
                &obj(json!({"runtime": "python", "version": "3.9.6", "mode": mode})),
            )
            .unwrap_err();
            assert!(
                format!("{e:#}").contains("禁止卸载") || format!("{e:#}").contains("forget"),
                "{mode}: {e:#}"
            );
        }
        // refused before anyone is asked
        assert_eq!(seen(&fake), 0);
        assert!(ctx.manager().unwrap().is_installed("python", "3.9.6"));
    }

    #[test]
    fn gui_not_running_is_a_clear_refusal() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(t.path());
        paths.ensure().unwrap();
        let ctx = Ctx {
            paths: Some(paths),
            approver: Arc::new(GuiApprover),
            client: Mutex::new(None),
            cli: "/x".into(),
        };
        fake_install(&ctx, "node", "20.11.1");
        set_policy(
            &ctx,
            Policy {
                mode: "allow_write".into(),
                ..Default::default()
            },
        );
        let e = call(
            &ctx,
            "uninstall",
            &obj(json!({"runtime": "node", "version": "20.11.1"})),
        )
        .unwrap_err();
        assert!(e.to_string().contains("请先打开 Runa"), "{e}");
        assert!(ctx.manager().unwrap().is_installed("node", "20.11.1"));
    }
}
