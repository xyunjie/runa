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
    },
    /// 卸载某个已安装版本
    #[command(alias = "rm")]
    Uninstall { tool: String, version: String },
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
    List { tool: Option<String> },
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
    /// 检测系统中已有的运行时
    Detect,
    /// 接管系统已有的运行时（只创建符号链接，不改动原安装）
    Adopt {
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

struct Bar {
    pb: Option<ProgressBar>,
}

impl Bar {
    fn new() -> Self {
        Self { pb: None }
    }
    fn handle(&mut self, p: Progress) {
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
    let mut m = Manager::load()?;
    let quiet = matches!(
        cli.cmd,
        Cmd::Env { .. } | Cmd::Activate { .. } | Cmd::Shim { .. }
    );
    if !quiet {
        for w in &m.warnings {
            eprintln!("警告: {w}");
        }
    }
    let here = cwd();
    match cli.cmd {
        Cmd::Install { tool: None, .. } => {
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
        } => {
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
                eprintln!("已将 {tool} {v} 设为全局默认版本（首次安装）");
            }
            println!("{v}");
        }
        Cmd::Uninstall { tool, version } => {
            let (tool, v2) = split_tool(&tool, Some(version));
            let version = v2.unwrap();
            m.uninstall(&tool, &version)?;
            reshim_quiet(&m);
            eprintln!("✓ 已卸载 {tool} {version}");
        }
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
        Cmd::List { tool } => {
            let tools = match tool {
                Some(t) => vec![m.plugin(&t)?.name().to_string()],
                None => m.registry.names(),
            };
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
                    let extra = match m.receipt(&t, &v) {
                        Some(r) if r.source == "adopted" => {
                            format!(
                                "  (已接管: {})",
                                r.origin
                                    .map(|o| o.display().to_string())
                                    .unwrap_or_default()
                            )
                        }
                        _ => String::new(),
                    };
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
        Cmd::Detect => {
            let list = detect::detect(&m);
            if list.is_empty() {
                eprintln!("没有检测到系统中已安装的运行时");
            }
            for d in list {
                let mark = if d.adopted { "（已接管）" } else { "" };
                println!(
                    "{:<8} {:<14} {}  [{}]{mark}",
                    d.tool,
                    d.version,
                    d.exe.display(),
                    d.via
                );
            }
        }
        Cmd::Adopt {
            tool,
            version,
            name,
        } => {
            let tool = tool
                .map(|t| m.plugin(&t).map(|p| p.name().to_string()))
                .transpose()?;
            let list: Vec<_> = detect::detect(&m)
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
                match detect::adopt(&m, &d, name.as_deref()) {
                    Ok(v) => eprintln!("✓ 已接管 {} {v} ← {}", d.tool, d.exe.display()),
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
            let cli = activate::cli_path();
            let p = activate::setup_profile(&m, &shell, &cli)?;
            activate::reshim(&m, &cli)?;
            if activate::add_shims_to_user_path(&m)? {
                eprintln!("✓ 已把 {} 加入用户 PATH", m.paths.shims().display());
            }
            eprintln!("✓ 已写入 {}，重新打开终端后生效", p.display());
        }
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

fn reshim_quiet(m: &Manager) {
    if let Err(e) = activate::reshim(m, &activate::cli_path()) {
        eprintln!("警告: 生成 shim 失败: {e:#}");
    }
}

/// Run `cmd` with the plan's PATH prepended and variables set.
fn run_with_plan(
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

fn spawn(
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
