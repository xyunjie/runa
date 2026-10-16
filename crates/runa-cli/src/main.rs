//! `runa` command line interface.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use runa_core::activate;
use runa_core::detect;
use runa_core::installer::Progress;
use runa_core::manager::find_exe_in;
use runa_core::Manager;

mod context;
mod extra;
mod mcp;

#[derive(Parser)]
#[command(
    name = "runa",
    version,
    about = "runa —— 多语言运行时版本管理器（Node / Java / Python / Go …）"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 安装版本：runa install node 20 / runa install python@3.12；不带参数则安装当前目录需要的全部版本
    Install {
        tool: Option<String>,
        version: Option<String>,
        /// 上游没有提供 SHA256 时拒绝安装（也可设置 RUNA_REQUIRE_CHECKSUM=1）
        #[arg(long)]
        require_checksum: bool,
    },
    /// 卸载某个已安装版本；接管的版本需选择 --forget（只移除管理）或 --purge（彻底卸载）
    #[command(
        alias = "rm",
        after_help = "示例:\n  runa uninstall node 20.11.1\n  runa uninstall java 21.0.11 --forget\n  runa uninstall node 24.13.0 --purge --dry-run"
    )]
    Uninstall {
        tool: String,
        version: String,
        /// 接管的版本：通过其所属工具彻底卸载（brew / nvm / pyenv / sdk / asdf / fnm，或删除目录）
        #[arg(long, conflicts_with = "forget")]
        purge: bool,
        /// 接管的版本：只移除 runa 的管理记录，保留原有文件
        #[arg(long)]
        forget: bool,
        /// 只打印将执行的命令和将删除的路径
        #[arg(long)]
        dry_run: bool,
    },
    /// 禁用某个版本（仍保留在列表中，但不会被解析、生成 shim 或加入 PATH）
    Disable {
        tool: String,
        version: Option<String>,
    },
    /// 重新启用被禁用的版本
    Enable {
        tool: String,
        version: Option<String>,
    },
    /// 设置版本：默认写入当前目录 .tool-versions，-g 写入全局
    Use {
        tool: String,
        version: String,
        /// 写入全局 ~/.runa/.tool-versions
        #[arg(short, long)]
        global: bool,
        /// 未安装时不要自动安装
        #[arg(long)]
        no_install: bool,
    },
    /// 列出已安装版本（* 表示当前目录生效的版本）
    #[command(alias = "ls")]
    List {
        tool: Option<String>,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 列出可安装的远程版本
    ListAll {
        tool: String,
        /// 只显示以此开头的版本，如 20
        prefix: Option<String>,
        /// 最多显示条数
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
    },
    /// 显示当前目录生效的版本及其来源
    Current { tool: Option<String> },
    /// 显示某个命令实际会执行的文件路径
    Which { command: String },
    /// 在指定版本环境中执行命令：runa exec --with node@20 -- node -v
    Exec {
        /// 临时指定版本（可多次），如 node@20
        #[arg(long = "with", short = 'w')]
        with: Vec<String>,
        #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// 重新生成 shims（安装全局 npm 包等之后执行）
    Reshim,
    /// 输出 shell 钩子脚本：eval "$(runa activate bash)"
    Activate { shell: Option<String> },
    /// 输出当前目录的环境变量设置脚本（供钩子调用）
    Env { shell: Option<String> },
    /// 检测系统中已有的运行时（默认使用缓存，机器状态变化或超过 24 小时自动重新扫描）
    Detect {
        /// 忽略缓存，重新扫描
        #[arg(long)]
        refresh: bool,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
    },
    /// 接管系统已有的运行时（只登记真实安装位置，不创建链接、不改动原安装）
    Adopt {
        /// 把旧版（符号链接方式）的接管记录迁移为登记方式
        #[arg(long, conflicts_with_all = ["tool", "version", "name"])]
        migrate: bool,
        /// 与 --migrate 一起使用：只显示将做的修改
        #[arg(long, requires = "migrate")]
        dry_run: bool,
        /// 只接管某个工具
        tool: Option<String>,
        /// 只接管该版本
        #[arg(long)]
        version: Option<String>,
        /// 接管后使用的版本名
        #[arg(long)]
        name: Option<String>,
    },
    /// 查看或设置下载镜像：runa mirror [off|cn]
    Mirror { mode: Option<String> },
    /// 查看或设置代理：runa proxy [off|system|http://..|socks5://..]
    Proxy { value: Option<String> },
    /// 一键把激活脚本写入 shell 配置文件
    Setup {
        /// 默认根据 $SHELL 判断；可选 bash / zsh / fish / powershell
        shell: Option<String>,
        /// 从配置文件中移除
        #[arg(long)]
        remove: bool,
    },
    /// 给 Agent / AI 的只读版本上下文：项目要求、解析与已激活版本、冲突与建议命令
    #[command(
        after_help = "示例:\n  runa context\n  runa context --json --check   # 未就绪时退出码为 1"
    )]
    Context {
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
        /// 环境不满足项目要求时返回非零退出码
        #[arg(long)]
        check: bool,
    },
    /// 按项目声明的版本执行命令（docker 模式下在容器中执行）
    #[command(
        after_help = "示例:\n  runa run -- node -v\n  runa run --install-missing -- npm test"
    )]
    Run {
        /// 先安装缺失的版本（强制要求上游 SHA256）
        #[arg(long)]
        install_missing: bool,
        /// 兼容参数：run 补装时总是要求 SHA256
        #[arg(long, hide = true)]
        require_checksum: bool,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// 项目环境：一个项目一个环境（local 隔离 / versions 共享 / docker 容器）
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// MCP 服务（stdio）或生成 AI 客户端配置，固定绑定一个项目目录
    #[command(
        after_help = "示例:\n  runa mcp serve --project .\n  runa mcp config --project . --client cursor\n  runa mcp config --project . --client codex --read-only\n\n客户端: cursor / claude-desktop / claude-code / codex / vscode / windsurf / generic"
    )]
    Mcp {
        /// serve（默认）或 config
        #[arg(value_parser = ["serve", "config"])]
        op: Option<String>,
        /// 绑定的项目目录（必填）
        #[arg(long)]
        project: Option<PathBuf>,
        /// 只暴露只读工具
        #[arg(long)]
        read_only: bool,
        /// config 的目标客户端
        #[arg(long)]
        client: Option<String>,
    },
    /// 打开临时子 shell 使用某版本（不写任何配置，exit 即恢复）
    Shell {
        tool: String,
        version: String,
        /// bash / zsh / fish / powershell，默认当前 shell
        shell: Option<String>,
    },
    /// 输出 shell 补全脚本：eval "$(runa completion bash)"
    Completion { shell: clap_complete::Shell },
    /// 统计各版本占用的磁盘空间
    Size {
        tool: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 清理下载缓存
    Clean {
        #[arg(long)]
        json: bool,
    },
    /// 把 shims / 激活脚本写入 PATH（Windows 用户 PATH；macOS / Linux 写 shell 配置），同 setup
    PathInit { shell: Option<String> },
    /// 从 GitHub Releases 检查并更新 runa（SHA256 校验）
    #[command(
        after_help = "示例:\n  runa update --check\n  runa update\n  runa update --version v0.2.0"
    )]
    Update {
        /// 只检查，不更新
        #[arg(long)]
        check: bool,
        /// 更新到指定的稳定版本
        #[arg(long)]
        version: Option<String>,
        /// 同时更新桌面端（不支持：桌面端以安装包发布）
        #[arg(long, hide = true)]
        gui: bool,
        #[arg(long)]
        json: bool,
    },
    /// 列出所有可管理的工具（含 JSON 插件）
    Plugins,
    /// 诊断信息
    Doctor,
    /// shim 入口（内部使用）
    #[command(hide = true)]
    Shim {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// 初始化项目环境（默认 local：版本与缓存都放在 <项目>/.runa/env）
    Init {
        #[arg(long, default_value = "local", value_parser = ["local", "versions", "docker"])]
        mode: String,
        /// docker 模式使用的镜像，如 node:22-bookworm
        #[arg(long)]
        image: Option<String>,
    },
    /// 查看项目环境与隔离模式
    Status {
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("错误: {e:#}");
            std::process::exit(1);
        }
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// `node@20` -> ("node", Some("20"))
fn split_tool(tool: &str, version: Option<String>) -> (String, Option<String>) {
    match tool.split_once('@') {
        Some((t, v)) => (t.to_string(), Some(v.to_string())),
        None => (tool.to_string(), version),
    }
}

pub(crate) struct Bar {
    pb: Option<ProgressBar>,
}

impl Bar {
    pub(crate) fn new() -> Self {
        Self { pb: None }
    }
    pub(crate) fn handle(&mut self, p: Progress) {
        match p {
            Progress::Resolving { tool, spec } => eprintln!("→ 解析 {tool} {spec} …"),
            Progress::Downloading {
                tool,
                version,
                done,
                total,
            } => {
                let pb = self.pb.get_or_insert_with(|| {
                    let pb = ProgressBar::new(total.unwrap_or(0));
                    pb.set_style(
                        ProgressStyle::with_template(
                            "  下载 {msg} [{bar:30}] {bytes}/{total_bytes} {bytes_per_sec}",
                        )
                        .unwrap()
                        .progress_chars("=> "),
                    );
                    pb.set_message(format!("{tool} {version}"));
                    pb
                });
                if let Some(t) = total {
                    pb.set_length(t);
                }
                pb.set_position(done);
            }
            Progress::Verifying { checksum, .. } => {
                if let Some(pb) = self.pb.take() {
                    pb.finish_and_clear();
                }
                if checksum {
                    eprintln!("  ✓ SHA256 校验通过");
                }
            }
            Progress::Extracting { .. } => eprintln!("  解压中 …"),
            Progress::Done { tool, version } => eprintln!("✓ {tool} {version} 已就绪"),
            Progress::Warning { message } => eprintln!("  ! {message}"),
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    // `env` / `activate` / `shim` run on every prompt or command: keep them quiet.
    let here = cwd();
    // Commands that do not need (or must not fail on) a project manager.
    match &cli.cmd {
        Cmd::Completion { shell } => {
            extra::completion(*shell);
            return Ok(0);
        }
        Cmd::Mcp {
            op,
            project,
            read_only,
            client,
        } => {
            return mcp::command(
                op.as_deref().unwrap_or("serve"),
                project.as_deref(),
                *read_only,
                client.as_deref(),
            );
        }
        _ => {}
    }
    let mut m = Manager::load_for(&here)?;
    if let Ok(expect) = std::env::var("RUNA_MCP_EXPECT_MODE") {
        let got = m.project.as_ref().map(|p| p.mode.as_str()).unwrap_or("");
        if !expect.is_empty() && expect != got {
            bail!("项目环境模式已变化（期望 {expect}），拒绝执行");
        }
    }
    let quiet = matches!(
        cli.cmd,
        Cmd::Env { .. } | Cmd::Activate { .. } | Cmd::Shim { .. }
    );
    if !quiet {
        for w in &m.warnings {
            eprintln!("警告: {w}");
        }
    }
    match cli.cmd {
        Cmd::Install {
            tool: None,
            require_checksum,
            ..
        } => {
            m.require_checksum = require_checksum;
            let mut any = false;
            for r in m.resolve_all(&here) {
                if r.version.is_none() && !r.system {
                    any = true;
                    let mut bar = Bar::new();
                    let spec = r.wanted().to_string();
                    m.install(&r.tool, &spec, &mut |p| bar.handle(p))?;
                }
            }
            if !any {
                eprintln!("当前目录需要的版本均已安装");
            }
            reshim_quiet(&m);
        }
        Cmd::Install {
            tool: Some(tool),
            version,
            require_checksum,
        } => {
            m.require_checksum = require_checksum;
            let (tool, version) = split_tool(&tool, version);
            let spec = match version {
                Some(v) => v,
                None => m
                    .resolve(&tool, &here)
                    .map(|r| r.wanted().to_string())
                    .unwrap_or_else(|| "latest".into()),
            };
            let mut bar = Bar::new();
            let v = m.install(&tool, &spec, &mut |p| bar.handle(p))?;
            reshim_quiet(&m);
            if m.global_version(&tool).is_none() {
                m.set_global(&tool, &v)?;
                if m.paths.is_isolated() {
                    eprintln!("已将 {tool} {v} 设为项目环境的默认版本（首次安装）");
                } else {
                    eprintln!("已将 {tool} {v} 设为全局默认版本（首次安装）");
                }
            }
            println!("{v}");
        }
        Cmd::Uninstall {
            tool,
            version,
            purge,
            forget,
            dry_run,
        } => {
            let (tool, v2) = split_tool(&tool, Some(version));
            return uninstall_cmd(&m, &tool, &v2.unwrap(), purge, forget, dry_run);
        }
        Cmd::Disable { tool, version } => toggle(&m, &here, &tool, version, true)?,
        Cmd::Enable { tool, version } => toggle(&m, &here, &tool, version, false)?,
        Cmd::Use {
            tool,
            version,
            global,
            no_install,
        } => {
            let (tool, version) = split_tool(&tool, Some(version));
            let spec = version.unwrap();
            let v = if spec == "system" {
                spec.clone()
            } else if no_install {
                m.match_installed(&tool, &spec)
                    .ok_or_else(|| anyhow!("{tool} {spec} 未安装"))?
            } else {
                let mut bar = Bar::new();
                m.ensure_installed(&tool, &spec, &mut |p| bar.handle(p))?
            };
            let tool = m.plugin(&tool)?.name().to_string();
            let file = if global {
                m.set_global(&tool, &v)?
            } else {
                m.set_local(&here, &tool, &v)?
            };
            reshim_quiet(&m);
            eprintln!("✓ {tool} {v} → {}", file.display());
            if std::env::var_os("_RUNA_ENV_KEYS").is_none()
                && std::env::var_os("RUNA_PATH_BASE").is_none()
            {
                eprintln!(
                    "提示: 当前 shell 尚未激活 runa，运行 `runa setup` 后重开终端即可自动切换"
                );
            }
        }
        Cmd::List { tool, json } => {
            let tools = match tool {
                Some(t) => vec![m.plugin(&t)?.name().to_string()],
                None => m.registry.names(),
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&extra::list_json(&m, &tools, &here))?
                );
                return Ok(0);
            }
            for t in tools {
                let installed = m.installed(&t);
                if installed.is_empty() {
                    continue;
                }
                let cur = m.resolve(&t, &here).and_then(|r| r.version);
                println!("{t}");
                for v in installed {
                    let mark = if cur.as_deref() == Some(v.as_str()) {
                        "*"
                    } else {
                        " "
                    };
                    let r = m.receipt(&t, &v);
                    let mut extra = match &r {
                        Some(r) if r.is_adopted() => format!(
                            "  (已接管{}: {})",
                            r.provider
                                .as_deref()
                                .map(|p| format!(" · {p}"))
                                .unwrap_or_default(),
                            m.runtime_root(&t, &v).display()
                        ),
                        _ => String::new(),
                    };
                    if r.map(|r| r.disabled).unwrap_or(false) {
                        extra.push_str("  [已禁用]");
                    }
                    println!("  {mark} {v}{extra}");
                }
            }
        }
        Cmd::ListAll {
            tool,
            prefix,
            limit,
        } => {
            let p = m.plugin(&tool)?;
            let net = m.net()?;
            let all = p.list_all(&net)?;
            let filtered: Vec<&String> = match &prefix {
                Some(pre) => {
                    let req =
                        runa_core::version::VersionReq::Prefix(runa_core::version::components(pre));
                    all.iter()
                        .filter(|v| {
                            v.starts_with(pre.as_str())
                                && (req.matches(v) || v.contains(|c: char| c.is_alphabetic()))
                        })
                        .collect()
                }
                None => all.iter().collect(),
            };
            let installed = m.installed(p.name());
            for v in filtered.iter().take(limit) {
                let mark = if installed.contains(v) {
                    " (已安装)"
                } else {
                    ""
                };
                println!("{v}{mark}");
            }
            if filtered.len() > limit {
                eprintln!("… 共 {} 个版本，使用 --limit 查看更多", filtered.len());
            }
        }
        Cmd::Current { tool } => {
            let list = match tool {
                Some(t) => {
                    let t = m.plugin(&t)?.name().to_string();
                    m.resolve(&t, &here).into_iter().collect::<Vec<_>>()
                }
                None => m.resolve_all(&here),
            };
            if list.is_empty() {
                eprintln!("当前目录没有设置任何版本（使用 runa use <工具> <版本> 设置）");
            }
            for r in list {
                let state = match (&r.version, r.system) {
                    (_, true) => "system".to_string(),
                    (Some(v), _) => v.clone(),
                    (None, _) => format!("{}（未安装，运行 runa install）", r.wanted()),
                };
                println!("{:<8} {:<22} {}", r.tool, state, r.spec.source.describe());
            }
        }
        Cmd::Which { command } => match m.which(&command, &here)? {
            Some(p) => println!("{}", p.display()),
            None => bail!("找不到命令 {command}"),
        },
        Cmd::Exec { with, command } => {
            let overrides: Vec<(String, String)> = with
                .iter()
                .map(|w| {
                    let (t, v) = split_tool(w, None);
                    (t, v.unwrap_or_else(|| "latest".into()))
                })
                .collect();
            let plan = m.env_plan(&here, &overrides)?;
            return run_with_plan(&m, &plan, &command[0], &command[1..], false);
        }
        Cmd::Reshim => {
            let cli = activate::cli_path();
            let names = activate::reshim(&m, &cli)?;
            eprintln!(
                "✓ 已生成 {} 个 shim 于 {}",
                names.len(),
                m.paths.shims().display()
            );
        }
        Cmd::Activate { shell } => {
            let shell = shell.unwrap_or_else(activate::default_shell);
            print!("{}", activate::hook_script(&shell, &activate::cli_path())?);
        }
        Cmd::Env { shell } => {
            let shell = shell.unwrap_or_else(activate::default_shell);
            print!("{}", activate::env_script(&m, &shell, &here)?);
        }
        Cmd::Detect { refresh, json } => {
            let rep = if refresh {
                detect::detect_fresh(&m)
            } else {
                detect::detect_cached(&m)
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
                return Ok(0);
            }
            if rep.items.is_empty() {
                eprintln!("没有检测到系统中已安装的运行时");
            }
            for d in &rep.items {
                let mut mark = match (&d.adopted_as, d.adopted) {
                    (Some(v), true) if v != &d.version => format!("（已接管为 {v}）"),
                    (_, true) => "（已接管）".to_string(),
                    _ => String::new(),
                };
                if d.disabled {
                    mark.push_str("（已禁用）");
                }
                if d.system_default {
                    mark.push_str("（当前全局）");
                }
                if d.runa_global {
                    mark.push_str("（runa 全局）");
                }
                let provider = match &d.formula {
                    Some(f) => format!("{}:{f}", d.provider),
                    None => d.provider.clone(),
                };
                println!(
                    "{:<7} {:<10} {:<20} {}  [{}]{mark}",
                    d.tool,
                    d.version,
                    provider,
                    d.home.display(),
                    d.via
                );
            }
            if rep.from_cache {
                eprintln!("（来自缓存，使用 --refresh 重新扫描）");
            }
        }
        Cmd::Adopt {
            migrate: true,
            dry_run,
            ..
        } => {
            let acts = detect::migrate(&m, dry_run)?;
            if acts.is_empty() {
                eprintln!("没有需要迁移的旧接管记录");
            }
            for a in &acts {
                let verb = match (a.action.as_str(), dry_run) {
                    ("convert", true) => "将转换",
                    ("convert", false) => "已转换",
                    (_, true) => "将删除记录",
                    (_, false) => "已删除记录",
                };
                println!(
                    "{verb} {} {}: {} → {}{}",
                    a.tool,
                    a.version,
                    a.old_origin
                        .as_deref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    a.home
                        .as_deref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "-".into()),
                    a.provider
                        .as_deref()
                        .map(|p| format!(" [{p}]"))
                        .unwrap_or_default()
                );
                println!("    {}", a.note);
                for p in &a.removes {
                    println!(
                        "    {}删除 {}",
                        if dry_run { "将" } else { "已" },
                        p.display()
                    );
                }
            }
            if !dry_run && !acts.is_empty() {
                reshim_quiet(&m);
            }
        }
        Cmd::Adopt {
            tool,
            version,
            name,
            ..
        } => {
            let tool = tool
                .map(|t| m.plugin(&t).map(|p| p.name().to_string()))
                .transpose()?;
            let list: Vec<_> = detect::detect_cached(&m)
                .items
                .into_iter()
                .filter(|d| !d.adopted)
                .filter(|d| tool.as_deref().map(|t| t == d.tool).unwrap_or(true))
                .filter(|d| version.as_deref().map(|v| v == d.version).unwrap_or(true))
                .collect();
            if list.is_empty() {
                eprintln!("没有可接管的运行时（先运行 runa detect 查看）");
            }
            if name.is_some() && list.len() > 1 {
                bail!("--name 只能用于单个运行时，请用 tool / --version 缩小范围");
            }
            for d in list {
                match detect::adopt_detected(&m, &d, name.as_deref()) {
                    Ok((v, global)) => eprintln!(
                        "✓ 已接管 {} {v} ← {} [{}]{}",
                        d.tool,
                        d.home.display(),
                        d.provider,
                        if global {
                            "，已设为全局版本"
                        } else {
                            ""
                        }
                    ),
                    Err(e) => eprintln!("✗ {} {}: {e:#}", d.tool, d.version),
                }
            }
            reshim_quiet(&m);
        }
        Cmd::Mirror { mode } => {
            if let Some(mode) = mode {
                m.settings.set_mirror(&mode)?;
                m.save_settings()?;
                eprintln!("✓ 镜像已设置为 {}", m.settings.mirror);
            }
            println!("镜像: {}", m.settings.mirror);
            for r in m.settings.rules() {
                println!("  {} → {}", r.from, r.to);
            }
        }
        Cmd::Proxy { value } => {
            if let Some(v) = value {
                m.settings.set_proxy(&v)?;
                m.save_settings()?;
                eprintln!("✓ 代理已设置为 {}", m.settings.proxy);
            }
            println!("代理: {}", m.settings.proxy);
        }
        Cmd::Setup { shell, remove } => {
            let shell = activate::normalize_shell(&shell.unwrap_or_else(activate::default_shell))?;
            if remove {
                match activate::remove_profile(&shell)? {
                    Some(p) => eprintln!("✓ 已从 {} 移除 runa", p.display()),
                    None => eprintln!("配置文件中没有 runa 配置"),
                }
                return Ok(0);
            }
            setup(&m, Some(shell))?;
        }
        Cmd::Context { json, check } => return context::command(&m, &here, json, check),
        Cmd::Run {
            install_missing,
            require_checksum: _,
            command,
        } => return extra::run(&mut m, &here, install_missing, &command),
        Cmd::Project { cmd } => match cmd {
            ProjectCmd::Init { mode, image } => {
                let p = runa_core::project::init(&here, &mode, image.as_deref())?;
                println!("{}", serde_json::to_string_pretty(&p)?);
                if p.mode == "local" {
                    eprintln!(
                        "✓ 项目环境已初始化：版本将安装到 {}",
                        p.env_root().display()
                    );
                }
            }
            ProjectCmd::Status { json } => {
                let p = m.project.clone();
                if json {
                    let v = serde_json::json!({
                        "initialized": p.is_some(),
                        "project": p,
                        "installs": p.as_ref().map(|_| m.paths.installs()),
                    });
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    match p {
                        None => {
                            println!("项目环境尚未初始化；执行 runa project init（默认 local）")
                        }
                        Some(p) => {
                            println!("项目: {}\n隔离模式: {}", p.root().display(), p.mode);
                            match &p.image {
                                Some(i) => println!("容器镜像: {i}"),
                                None => println!("工具目录: {}", m.paths.installs().display()),
                            }
                        }
                    }
                }
            }
        },
        Cmd::Shell {
            tool,
            version,
            shell,
        } => return extra::shell(&m, &here, &tool, &version, shell.as_deref()),
        Cmd::Size { tool, json } => extra::size(&m, tool.as_deref(), json)?,
        Cmd::Clean { json } => extra::clean(&m, json)?,
        Cmd::PathInit { shell } => {
            if m.paths.is_isolated() {
                bail!("隔离项目请使用 runa run 或 shell 钩子，不把项目目录持久写入 PATH");
            }
            setup(&m, shell)?;
        }
        Cmd::Update {
            check,
            version,
            gui,
            json,
        } => {
            if gui {
                bail!("桌面端以安装包（.msi / .dmg / .deb / .AppImage）发布，请从 GitHub Releases 下载更新；runa update 只更新命令行");
            }
            return extra::update(&m, check, version.as_deref(), json);
        }
        Cmd::Completion { .. } | Cmd::Mcp { .. } => unreachable!(),
        Cmd::Plugins => {
            for p in m.registry.all() {
                let kind = if p.builtin() { "内置" } else { "JSON 插件" };
                println!("{:<8} {:<10} {}", p.name(), kind, p.description());
            }
        }
        Cmd::Doctor => {
            println!("runa 版本:   {}", env!("CARGO_PKG_VERSION"));
            println!("RUNA_HOME:   {}", m.paths.root.display());
            println!("CLI 路径:    {}", activate::cli_path().display());
            println!("平台:        {}", runa_core::Target::current().key());
            println!("镜像:        {}", m.settings.mirror);
            println!("代理:        {}", m.settings.proxy);
            let active = std::env::var_os("RUNA_PATH_BASE").is_some();
            println!(
                "shell 钩子:  {}",
                if active {
                    "已激活"
                } else {
                    "未激活（运行 runa setup）"
                }
            );
            let path = std::env::var_os("PATH").unwrap_or_default();
            let shims_on_path = std::env::split_paths(&path).any(|p| p == m.paths.shims());
            println!("shims 在 PATH: {}", if shims_on_path { "是" } else { "否" });
            for sh in activate::SHELLS {
                if let Ok(p) = activate::profile_path(sh) {
                    let ok = activate::profile_has_block(sh);
                    println!("  {sh:<10} {} {}", if ok { "✓" } else { "-" }, p.display());
                }
            }
        }
        Cmd::Shim { name, args } => {
            if std::env::var_os("RUNA_DISABLE_SHIM").is_some() {
                // never recurse into ourselves during detection
                let sys = m.system_path();
                let exe = find_exe_in(&sys, &name).ok_or_else(|| anyhow!("找不到命令 {name}"))?;
                return spawn(&exe, &args, &[], None);
            }
            let plan = m.env_plan(&here, &[])?;
            if find_exe_in(&plan.paths, &name).is_none()
                && find_exe_in(&m.system_path(), &name).is_none()
            {
                let providers = m.providers_of(&name);
                if providers.is_empty() {
                    bail!("找不到命令 {name}");
                }
                let list: Vec<String> = providers.iter().map(|(t, v)| format!("{t} {v}")).collect();
                bail!(
                    "当前目录没有为 `{name}` 设置版本。已安装: {}\n  运行 `runa use {} <版本>`（加 -g 设为全局）",
                    list.join(", "),
                    providers[0].0
                );
            }
            return run_with_plan(&m, &plan, &name, &args, true);
        }
    }
    Ok(0)
}

/// Shell profile + shims always use the global home, even inside an isolated project.
fn setup(_m: &Manager, shell: Option<String>) -> Result<()> {
    let global = Manager::load()?;
    let m = &global;
    let shell = activate::normalize_shell(&shell.unwrap_or_else(activate::default_shell))?;
    let cli = activate::cli_path();
    let p = activate::setup_profile(m, &shell, &cli)?;
    activate::reshim(m, &cli)?;
    if activate::add_shims_to_user_path(m)? {
        eprintln!("✓ 已把 {} 加入用户 PATH", m.paths.shims().display());
    }
    eprintln!("✓ 已写入 {}，重新打开终端后生效", p.display());
    Ok(())
}

fn toggle(
    m: &Manager,
    here: &Path,
    tool: &str,
    version: Option<String>,
    disable: bool,
) -> Result<()> {
    let verb = if disable { "disable" } else { "enable" };
    let (tool, version) = split_tool(tool, version);
    let version = version.ok_or_else(|| anyhow!("用法: runa {verb} <工具> <版本>"))?;
    let tool = m.plugin(&tool)?.name().to_string();
    for w in m.set_disabled(&tool, &version, disable, here)? {
        eprintln!("警告: {w}");
    }
    reshim_quiet(m);
    eprintln!(
        "✓ 已{} {tool} {version}",
        if disable { "禁用" } else { "启用" }
    );
    Ok(())
}

fn print_plan(p: &runa_core::uninstall::UninstallPlan) {
    println!("  {}", p.summary);
    if let Some(c) = &p.command_line {
        println!("  命令: {c}");
    }
    for path in &p.paths {
        println!("  删除: {}", path.display());
    }
    if p.needs_root {
        println!("  需要管理员权限（runa 不会自动 sudo）");
    }
}

fn uninstall_cmd(
    m: &Manager,
    tool: &str,
    version: &str,
    purge: bool,
    forget: bool,
    dry_run: bool,
) -> Result<i32> {
    use runa_core::uninstall::{self, Mode, PlanKind};
    let tool = m.plugin(tool)?.name().to_string();
    let adopted = m
        .receipt(&tool, version)
        .map(|r| r.is_adopted())
        .unwrap_or(false);
    let mode = match (purge, forget) {
        (true, _) => Mode::Purge,
        (_, true) => Mode::Forget,
        _ => Mode::Auto,
    };
    if adopted && mode == Mode::Auto {
        eprintln!("{tool} {version} 是接管的版本，请明确选择：");
        for (flag, md) in [("--forget", Mode::Forget), ("--purge", Mode::Purge)] {
            println!("runa uninstall {tool} {version} {flag}");
            match uninstall::plan(m, &tool, version, md) {
                Ok(p) => print_plan(&p),
                Err(e) => println!("  不可用: {e:#}"),
            }
        }
        return Ok(2);
    }
    let plan = uninstall::plan(m, &tool, version, mode)?;
    if dry_run {
        println!("[dry-run] runa uninstall {tool} {version}");
        print_plan(&plan);
        return Ok(if plan.kind == PlanKind::Refuse { 1 } else { 0 });
    }
    uninstall::execute(m, &plan)?;
    reshim_quiet(m);
    eprintln!(
        "✓ {} {tool} {version}",
        match plan.kind {
            PlanKind::Forget => "已移除管理（文件保留）",
            _ => "已卸载",
        }
    );
    Ok(0)
}

pub(crate) fn reshim_quiet(m: &Manager) {
    if let Err(e) = activate::reshim(m, &activate::cli_path()) {
        eprintln!("警告: 生成 shim 失败: {e:#}");
    }
}

/// Run `cmd` with the plan's PATH prepended and variables set.
pub(crate) fn run_with_plan(
    m: &Manager,
    plan: &runa_core::EnvPlan,
    cmd: &str,
    args: &[String],
    from_shim: bool,
) -> Result<i32> {
    let base = m.system_path();
    let mut dirs = plan.paths.clone();
    dirs.extend(base.iter().cloned());
    let exe = if Path::new(cmd).components().count() > 1 {
        PathBuf::from(cmd)
    } else {
        find_exe_in(&dirs, cmd).ok_or_else(|| anyhow!("找不到命令 {cmd}"))?
    };
    // Children keep the shims dir after the real bins so nested calls still resolve.
    let mut path_dirs = plan.paths.clone();
    let cur = std::env::var_os("PATH").unwrap_or_default();
    path_dirs.extend(std::env::split_paths(&cur).filter(|p| !plan.paths.contains(p)));
    let path = std::env::join_paths(path_dirs).context("PATH 无效")?;
    let _ = from_shim;
    spawn(&exe, args, &plan.vars, Some(path))
}

pub(crate) fn spawn(
    exe: &Path,
    args: &[String],
    vars: &[(String, String)],
    path: Option<std::ffi::OsString>,
) -> Result<i32> {
    let mut c = Command::new(exe);
    c.args(args);
    for (k, v) in vars {
        c.env(k, v);
    }
    if let Some(p) = path {
        c.env("PATH", p);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = c.exec();
        Err(anyhow!("执行 {} 失败: {err}", exe.display()))
    }
    #[cfg(not(unix))]
    {
        let st = c
            .status()
            .with_context(|| format!("执行 {} 失败", exe.display()))?;
        Ok(st.code().unwrap_or(1))
    }
}
