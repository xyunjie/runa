//! run / shell / size / clean / completion / update / list --json.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use clap::CommandFactory;
use runa_core::manager::{dir_size, find_exe_in};
use runa_core::{activate, project, updater, Manager};
use serde::Serialize;

use crate::{reshim_quiet, run_with_plan, Bar, Cli};

pub fn completion(shell: clap_complete::Shell) {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "runa", &mut std::io::stdout());
}

pub fn list_json(m: &Manager, tools: &[String], cwd: &Path) -> serde_json::Value {
    let mut out = vec![];
    for t in tools {
        let cur = m.resolve(t, cwd).and_then(|r| r.version);
        let versions: Vec<_> = m
            .installed(t)
            .into_iter()
            .map(|v| {
                let r = m.receipt(t, &v);
                serde_json::json!({
                    "version": v,
                    "current": cur.as_deref() == Some(v.as_str()),
                    "adopted": r.as_ref().map(|r| r.source == "adopted").unwrap_or(false),
                    "verified": r.as_ref().map(|r| r.verified).unwrap_or(false),
                    "disabled": r.as_ref().map(|r| r.disabled).unwrap_or(false),
                    "provider": r.as_ref().and_then(|r| r.provider.clone()),
                    "path": m.runtime_root(t, &v),
                })
            })
            .collect();
        out.push(serde_json::json!({ "tool": t, "versions": versions }));
    }
    serde_json::json!({ "installs": m.paths.installs(), "tools": out })
}

/// `runa run [--install-missing] -- cmd args`
pub fn run(m: &mut Manager, cwd: &Path, install_missing: bool, command: &[String]) -> Result<i32> {
    let (cmd, args) = command
        .split_first()
        .ok_or_else(|| anyhow!("用法: runa run [--install-missing] -- <命令> [参数...]"))?;
    if let Some(p) = m.project.clone() {
        if p.mode == "docker" {
            return run_docker(m, &p, cwd, cmd, args);
        }
    }
    if install_missing {
        m.require_checksum = true;
        let mut any = false;
        for tool in m.registry.names() {
            let spec = match project::requirement(&tool, cwd) {
                Some(s) => s.specs.first().cloned(),
                None => m.resolve(&tool, cwd).map(|r| r.wanted().to_string()),
            };
            let Some(spec) = spec.filter(|s| !s.is_empty() && s != "system") else {
                continue;
            };
            if m.match_installed(&tool, &spec).is_some() {
                continue;
            }
            let mut bar = Bar::new();
            m.install(&tool, &spec, &mut |p| bar.handle(p))?;
            any = true;
        }
        if any {
            reshim_quiet(m);
        }
    }
    if let Some(p) = &m.project {
        p.prepare()?;
    }
    let plan = m.env_plan(cwd, &[])?;
    for r in &plan.tools {
        if r.version.is_none() && !r.system {
            eprintln!(
                "警告: {} {} 未安装（运行 runa install，或使用 runa run --install-missing）",
                r.tool,
                r.wanted()
            );
        }
    }
    run_with_plan(m, &plan, cmd, args, false)
}

fn run_docker(
    m: &Manager,
    p: &runa_core::Project,
    cwd: &Path,
    cmd: &str,
    args: &[String],
) -> Result<i32> {
    p.validate_state_root()?;
    let docker = find_exe_in(&m.system_path(), "docker")
        .ok_or_else(|| anyhow!("Docker 模式需要安装并启动 Docker Desktop / Docker Engine"))?;
    project::docker_cache_dirs(p)?;
    let mut command = cmd.to_string();
    // Verify the image's toolchains before running project code; never silently use an
    // incompatible version from the image.
    for tool in m.registry.names() {
        let Some(spec) = project::requirement(&tool, cwd).and_then(|s| s.specs.first().cloned())
        else {
            continue;
        };
        let plugin = m.plugin(&tool)?;
        let req = runa_core::version::VersionReq::parse(&spec);
        let mut found = false;
        for exe in plugin.detect_exes() {
            let vargs: Vec<String> = plugin
                .version_args()
                .iter()
                .map(|s| s.to_string())
                .collect();
            let flags = project::docker_args(p, cwd, exe, &vargs)?;
            let Ok(out) = Command::new(&docker).args(&flags).output() else {
                continue;
            };
            if !out.status.success() {
                continue;
            }
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let Some(v) = plugin.parse_version_output(&text) else {
                continue;
            };
            if !req.matches(&v) {
                bail!("Docker 镜像中的 {tool} {v} 不满足项目要求 {spec}，请选择匹配的镜像");
            }
            found = true;
            if command == tool {
                command = exe.to_string();
            }
            break;
        }
        if !found {
            bail!("Docker 镜像缺少项目要求的 {tool} {spec}，请在自定义镜像中安装");
        }
    }
    let flags = project::docker_args(p, cwd, &command, args)?;
    let st = Command::new(&docker)
        .args(&flags)
        .status()
        .with_context(|| format!("执行 {} 失败", docker.display()))?;
    Ok(st.code().unwrap_or(1))
}

/// Executable for a shell name.
fn shell_program(shell: &str) -> &'static str {
    match shell {
        "zsh" => "zsh",
        "fish" => "fish",
        "powershell" => {
            if cfg!(windows) {
                "powershell"
            } else {
                "pwsh"
            }
        }
        _ => "bash",
    }
}

/// `runa shell <tool> <version>`: temporary sub-shell; nothing is written to disk.
/// Setting `RUNA_<TOOL>_VERSION` makes the shell hook (if active) honor the choice too.
pub fn shell(m: &Manager, cwd: &Path, tool: &str, version: &str, sh: Option<&str>) -> Result<i32> {
    let tool = m.plugin(tool)?.name().to_string();
    let resolved = m.match_installed(&tool, version).ok_or_else(|| {
        anyhow!("{tool} 没有匹配 {version:?} 的已安装版本；先执行 `runa install {tool} {version}`")
    })?;
    let shell = activate::normalize_shell(
        &sh.map(str::to_string)
            .unwrap_or_else(activate::default_shell),
    )?;
    let plan = m.env_plan(cwd, &[(tool.clone(), resolved.clone())])?;
    let mut dirs = plan.paths.clone();
    let cur = std::env::var_os("PATH").unwrap_or_default();
    dirs.extend(std::env::split_paths(&cur).filter(|p| !plan.paths.contains(p)));
    let path = std::env::join_paths(dirs).context("PATH 无效")?;
    let program = if cfg!(unix) && sh.is_none() {
        std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| shell_program(&shell).into())
    } else {
        shell_program(&shell).to_string()
    };
    eprintln!("进入 {tool} {resolved} 临时环境（{shell}）。输入 exit 退出，不影响其它终端。");
    let mut c = Command::new(&program);
    for (k, v) in &plan.vars {
        c.env(k, v);
    }
    c.env("PATH", path)
        .env(runa_core::resolver::env_key(&tool), &resolved)
        .env("RUNA_SHELL", format!("{tool}@{resolved}"));
    let st = c.status().with_context(|| format!("启动 {program} 失败"))?;
    Ok(st.code().unwrap_or(0))
}

#[derive(Serialize)]
struct SizeRow {
    tool: String,
    version: String,
    path: PathBuf,
    bytes: u64,
    adopted: bool,
}

pub fn human(n: u64) -> String {
    const U: f64 = 1024.0;
    if n < 1024 {
        return format!("{n}B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= U && i < 5 {
        v /= U;
        i += 1;
    }
    format!("{v:.1}{}B", ['B', 'K', 'M', 'G', 'T', 'P'][i])
}

/// Size and number of regular files below `p` (symlinks are not followed).
pub fn count_files(p: &Path) -> (u64, usize) {
    let mut size = 0;
    let mut n = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                size += e.metadata().map(|m| m.len()).unwrap_or(0);
                n += 1;
            }
        }
    }
    (size, n)
}

pub fn size(m: &Manager, tool: Option<&str>, json: bool) -> Result<()> {
    let tools = match tool {
        Some(t) => vec![m.plugin(t)?.name().to_string()],
        None => m.registry.names(),
    };
    let mut rows = vec![];
    let mut total = 0;
    for t in &tools {
        for v in m.installed(t) {
            let adopted = m
                .receipt(t, &v)
                .map(|r| r.source == "adopted")
                .unwrap_or(false);
            let path = m.paths.install_dir(t, &v);
            let bytes = dir_size(&path);
            if !adopted {
                total += bytes;
            }
            rows.push(SizeRow {
                tool: t.clone(),
                version: v,
                path,
                bytes,
                adopted,
            });
        }
    }
    rows.sort_by_key(|a| std::cmp::Reverse(a.bytes));
    let (dl, dln) = count_files(&m.paths.downloads());
    if json {
        let v = serde_json::json!({ "items": rows, "totalBytes": total, "downloadCacheBytes": dl });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("没有已安装的版本。");
    }
    for r in &rows {
        let mark = if r.adopted {
            "（已接管，不计入）"
        } else {
            ""
        };
        println!(
            "{:<8} {:<16} {:>10}  {}{mark}",
            r.tool,
            r.version,
            human(r.bytes),
            r.path.display()
        );
    }
    if !rows.is_empty() {
        println!("{}", "─".repeat(58));
        println!("共 {} 个版本，占用 {}", rows.len(), human(total));
    }
    if dln > 0 {
        println!(
            "另有下载缓存 {dln} 个文件 {}（`runa clean` 可清理）",
            human(dl)
        );
    }
    Ok(())
}

pub fn clean(m: &Manager, json: bool) -> Result<()> {
    let dir = m.paths.downloads();
    let (size, n) = count_files(&dir);
    if n > 0 {
        std::fs::remove_dir_all(&dir).with_context(|| format!("清理 {} 失败", dir.display()))?;
        std::fs::create_dir_all(&dir)?;
    }
    if json {
        println!(
            "{}",
            serde_json::json!({ "removed": n, "freedBytes": size })
        );
    } else if n == 0 {
        println!("下载缓存已是空的。");
    } else {
        println!("已清理 {n} 个下载缓存文件，释放 {}", human(size));
    }
    Ok(())
}

pub fn update(m: &Manager, check: bool, version: Option<&str>, json: bool) -> Result<i32> {
    let net = m.net()?;
    let current = env!("CARGO_PKG_VERSION");
    let info = match updater::check(&net, current, version) {
        Ok(i) => i,
        // `--check` is informational: "no release yet" is not a failure
        Err(e) if check && format!("{e:#}").contains("还没有") => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "current": current, "available": false, "message": format!("{e:#}") })
                );
            } else {
                println!("{e:#}；当前 v{current}");
            }
            return Ok(0);
        }
        Err(e) => return Err(e),
    };
    if check || (!info.available && version.is_none()) {
        if json {
            println!("{}", serde_json::to_string_pretty(&info)?);
        } else if info.available {
            println!(
                "发现新版本 {}（当前 v{current}）\n{}\n运行 `runa update` 更新",
                info.latest, info.url
            );
        } else {
            println!("已是最新版本 v{current}");
        }
        return Ok(0);
    }
    let exe = std::env::current_exe().context("无法确定当前程序路径")?;
    let mut pb: Option<indicatif::ProgressBar> = None;
    let applied = updater::apply(&net, &info, &exe, &mut |done, total| {
        let bar = pb.get_or_insert_with(|| indicatif::ProgressBar::new(total.unwrap_or(0)));
        bar.set_position(done);
    })?;
    if let Some(b) = pb {
        b.finish_and_clear();
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&applied)?);
    } else {
        eprintln!(
            "✓ 已更新到 {}（SHA256 校验通过）: {}",
            applied.version,
            applied.path.display()
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_sizes() {
        assert_eq!(human(10), "10B");
        assert_eq!(human(2048), "2.0KB");
        assert_eq!(human(5 * 1024 * 1024), "5.0MB");
    }

    #[test]
    fn counts_files() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("a")).unwrap();
        std::fs::write(t.path().join("a/x"), "1234").unwrap();
        std::fs::write(t.path().join("y"), "12").unwrap();
        assert_eq!(count_files(t.path()), (6, 2));
        assert_eq!(count_files(&t.path().join("missing")), (0, 0));
    }

    #[test]
    fn completion_scripts_render() {
        for sh in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::PowerShell,
        ] {
            let mut buf = vec![];
            clap_complete::generate(sh, &mut Cli::command(), "runa", &mut buf);
            let s = String::from_utf8(buf).unwrap();
            assert!(s.contains("context") && s.contains("mcp"), "{sh:?}");
        }
    }
}
