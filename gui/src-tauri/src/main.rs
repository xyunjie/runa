//! Runa desktop GUI (Tauri v2). All logic lives in `runa-core`; this file only exposes it
//! as IPC commands. Every command runs on a blocking worker thread because core uses
//! blocking HTTP.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use runa_core::activate;
use runa_core::detect::{self, Detected};
use runa_core::installer::Progress;
use runa_core::manager::ToolStatus;
use runa_core::{Manager, Settings};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))
}

fn manager() -> anyhow::Result<Manager> {
    Manager::load()
}

fn project_dir(dir: Option<String>) -> PathBuf {
    dir.filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
        .or_else(dirs_home)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

#[derive(Serialize)]
struct ShellInfo {
    shell: String,
    profile: String,
    configured: bool,
}

#[derive(Serialize)]
struct Overview {
    home: String,
    cli_path: String,
    cli_found: bool,
    project_dir: String,
    platform: String,
    settings: Settings,
    tools: Vec<ToolStatus>,
    shells: Vec<ShellInfo>,
    warnings: Vec<String>,
    is_windows: bool,
    default_shell: String,
    hook_active: bool,
    app_version: String,
}

#[tauri::command]
async fn overview(project: Option<String>) -> CmdResult<Overview> {
    blocking(move || {
        let m = manager()?;
        let dir = project_dir(project);
        let cli = activate::cli_path();
        let shells = activate::SHELLS
            .iter()
            .map(|s| ShellInfo {
                shell: s.to_string(),
                profile: activate::profile_path(s)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                configured: activate::profile_has_block(s),
            })
            .collect();
        Ok(Overview {
            home: m.paths.root.display().to_string(),
            cli_found: activate::cli_available(&cli),
            cli_path: cli.display().to_string(),
            project_dir: dir.display().to_string(),
            platform: runa_core::Target::current().key(),
            settings: m.settings.clone(),
            tools: m.status(&dir),
            shells,
            warnings: m.warnings.clone(),
            is_windows: cfg!(windows),
            default_shell: activate::default_shell(),
            hook_active: std::env::var_os("RUNA_PATH_BASE").is_some(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    })
    .await
}

#[tauri::command]
async fn remote_versions(tool: String) -> CmdResult<Vec<runa_core::plugins::RemoteVersion>> {
    blocking(move || {
        let m = manager()?;
        let p = m.plugin(&tool)?;
        p.list_remote(&m.net()?)
    })
    .await
}

#[derive(Serialize)]
struct Usage {
    tool: String,
    version: String,
    bytes: u64,
}

/// Disk usage of every install (computed lazily, can take a moment for JDKs).
#[tauri::command]
async fn disk_usage() -> CmdResult<Vec<Usage>> {
    blocking(|| {
        let m = manager()?;
        let mut out = Vec::new();
        for tool in m.registry.names() {
            for version in m.installed(&tool) {
                let bytes = runa_core::manager::dir_size(&m.paths.install_dir(&tool, &version));
                out.push(Usage {
                    tool: tool.clone(),
                    version,
                    bytes,
                });
            }
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
async fn install(app: AppHandle, tool: String, spec: String) -> CmdResult<String> {
    blocking(move || {
        let m = manager()?;
        let mut last = Instant::now() - Duration::from_secs(1);
        let v = m.install(&tool, &spec, &mut |p| {
            if let Progress::Downloading { done, total, .. } = &p {
                let finished = total.map(|t| *done >= t).unwrap_or(false);
                if last.elapsed() < Duration::from_millis(120) && !finished {
                    return;
                }
                last = Instant::now();
            }
            let _ = app.emit("install-progress", &p);
        })?;
        if m.global_version(&tool).is_none() {
            m.set_global(&tool, &v)?;
        }
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(v)
    })
    .await
}

#[tauri::command]
async fn uninstall(tool: String, version: String) -> CmdResult<()> {
    blocking(move || {
        let m = manager()?;
        m.uninstall(&tool, &version)?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(())
    })
    .await
}

#[tauri::command]
async fn set_global(tool: String, version: String) -> CmdResult<String> {
    blocking(move || {
        Ok(manager()?
            .set_global(&tool, &version)?
            .display()
            .to_string())
    })
    .await
}

#[tauri::command]
async fn set_local(project: String, tool: String, version: String) -> CmdResult<String> {
    blocking(move || {
        let dir = PathBuf::from(&project);
        if !dir.is_dir() {
            anyhow::bail!("项目目录不存在: {project}");
        }
        Ok(manager()?
            .set_local(&dir, &tool, &version)?
            .display()
            .to_string())
    })
    .await
}

#[tauri::command]
async fn unset_local(project: String, tool: String) -> CmdResult<()> {
    blocking(move || manager()?.unset(&PathBuf::from(project).join(".tool-versions"), &tool)).await
}

#[tauri::command]
async fn detect_runtimes() -> CmdResult<Vec<Detected>> {
    blocking(|| Ok(detect::detect(&manager()?))).await
}

#[tauri::command]
async fn adopt(item: Detected, name: Option<String>) -> CmdResult<String> {
    blocking(move || {
        let m = manager()?;
        let v = detect::adopt(&m, &item, name.as_deref().filter(|s| !s.is_empty()))?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(v)
    })
    .await
}

#[tauri::command]
async fn set_mirror(mode: String) -> CmdResult<Settings> {
    blocking(move || {
        let mut m = manager()?;
        m.settings.set_mirror(&mode)?;
        m.save_settings()?;
        Ok(m.settings.clone())
    })
    .await
}

#[tauri::command]
async fn set_proxy(value: String) -> CmdResult<Settings> {
    blocking(move || {
        let mut m = manager()?;
        m.settings.set_proxy(&value)?;
        m.save_settings()?;
        Ok(m.settings.clone())
    })
    .await
}

#[tauri::command]
async fn setup_shell(shell: String) -> CmdResult<String> {
    blocking(move || {
        let m = manager()?;
        let cli = activate::cli_path();
        if !activate::cli_available(&cli) {
            anyhow::bail!("未找到 runa 命令行程序，请先把 runa 放到 PATH 中或与 GUI 同目录");
        }
        let p = activate::setup_profile(&m, &shell, &cli)?;
        activate::reshim(&m, &cli)?;
        Ok(p.display().to_string())
    })
    .await
}

#[tauri::command]
async fn remove_shell(shell: String) -> CmdResult<Option<String>> {
    blocking(move || Ok(activate::remove_profile(&shell)?.map(|p| p.display().to_string()))).await
}

#[tauri::command]
async fn add_user_path() -> CmdResult<bool> {
    blocking(|| {
        let m = manager()?;
        activate::reshim(&m, &activate::cli_path())?;
        activate::add_shims_to_user_path(&m)
    })
    .await
}

#[tauri::command]
async fn reshim() -> CmdResult<usize> {
    blocking(|| Ok(activate::reshim(&manager()?, &activate::cli_path())?.len())).await
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            overview,
            remote_versions,
            disk_usage,
            install,
            uninstall,
            set_global,
            set_local,
            unset_local,
            detect_runtimes,
            adopt,
            set_mirror,
            set_proxy,
            setup_shell,
            remove_shell,
            add_user_path,
            reshim
        ])
        .run(tauri::generate_context!())
        .expect("启动 Runa 失败");
}
