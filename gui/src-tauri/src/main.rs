//! Runa desktop GUI (Tauri v2). All logic lives in `runa-core`; this file only exposes it
//! as IPC commands. Every command runs on a blocking worker thread because core uses
//! blocking HTTP.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use runa_core::activate;
use std::sync::atomic::{AtomicBool, Ordering};

use runa_core::detect::{self, DetectReport, Detected, MigrationAction};
use runa_core::installer::Progress;
use runa_core::manager::ToolStatus;
use runa_core::uninstall::{self, Mode, UninstallPlan};
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

#[derive(serde::Serialize)]
struct InstallResult {
    version: String,
    /// `runa` or the version manager that installed it.
    via: String,
}

#[tauri::command]
async fn install(
    app: AppHandle,
    tool: String,
    spec: String,
    via: Option<String>,
) -> CmdResult<InstallResult> {
    blocking(move || {
        let m = manager()?;
        let mut last = Instant::now() - Duration::from_secs(1);
        let (v, used) = runa_core::managers::install(&m, &tool, &spec, via.as_deref(), &mut |p| {
            let throttled = matches!(&p, Progress::Downloading { done, total, .. } if !total.map(|t| *done >= t).unwrap_or(false))
                || matches!(&p, Progress::Output { .. });
            if throttled && last.elapsed() < Duration::from_millis(120) {
                return;
            }
            last = Instant::now();
            let _ = app.emit("install-progress", &p);
        })?;
        if m.global_version(&tool).is_none() {
            m.set_global(&tool, &v)?;
        }
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(InstallResult { version: v, via: used })
    })
    .await
}

/// Which installer a new version of `tool` would use (and the alternatives).
#[tauri::command]
async fn installer_choice(tool: String) -> CmdResult<runa_core::managers::InstallerChoice> {
    blocking(move || {
        let m = manager()?;
        let t = m.plugin(&tool)?.name().to_string();
        Ok(runa_core::managers::installer_choice(
            &runa_core::managers::Probe::current(&m),
            &t,
            m.settings.install_via(&t),
        ))
    })
    .await
}

#[tauri::command]
async fn set_install_via(
    tool: String,
    via: String,
) -> CmdResult<runa_core::managers::InstallerChoice> {
    blocking(move || {
        let mut m = manager()?;
        let t = m.plugin(&tool)?.name().to_string();
        let probe = runa_core::managers::Probe::current(&m);
        if via != "auto" && via != "runa" {
            runa_core::managers::choose(&probe, &t, &via)?;
        }
        m.settings.set_install_via(&t, &via)?;
        m.save_settings()?;
        Ok(runa_core::managers::installer_choice(
            &probe,
            &t,
            m.settings.install_via(&t),
        ))
    })
    .await
}

/// What uninstalling would do (shown in the confirmation dialog). Changes nothing.
#[tauri::command]
async fn uninstall_plan(tool: String, version: String, mode: Mode) -> CmdResult<UninstallPlan> {
    blocking(move || uninstall::plan(&manager()?, &tool, &version, mode)).await
}

/// `mode`: `auto` (downloads), `forget` (adopted: remove management only) or `purge`
/// (adopted: complete uninstall through the owning tool).
#[tauri::command]
async fn uninstall(tool: String, version: String, mode: Option<Mode>) -> CmdResult<()> {
    blocking(move || {
        let m = manager()?;
        let plan = uninstall::plan(&m, &tool, &version, mode.unwrap_or(Mode::Auto))?;
        uninstall::execute(&m, &plan)?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(())
    })
    .await
}

/// Enable / disable a version; returns warnings (global version, project requirement).
#[tauri::command]
async fn set_disabled(
    tool: String,
    version: String,
    disabled: bool,
    project: Option<String>,
) -> CmdResult<Vec<String>> {
    blocking(move || {
        let m = manager()?;
        let w = m.set_disabled(&tool, &version, disabled, &project_dir(project))?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(w)
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
    blocking(|| Ok(detect::detect_cached(&manager()?).items)).await
}

/// Cached results immediately (no scan unless there is no cache at all). `stale` tells
/// the page to expect a `detect-updated` event from [`detect_check`].
#[tauri::command]
async fn detect_cached() -> CmdResult<DetectReport> {
    blocking(|| {
        let m = manager()?;
        Ok(detect::cached_only(&m).unwrap_or_else(|| detect::detect_cached(&m)))
    })
    .await
}

/// Force a rescan.
#[tauri::command]
async fn detect_refresh() -> CmdResult<DetectReport> {
    blocking(|| Ok(detect::detect_fresh(&manager()?))).await
}

static CHECKING: AtomicBool = AtomicBool::new(false);

/// Background check (app start / window focus): rescan only if the fingerprint changed
/// or the cache expired, then emit `detect-updated`. Returns whether it rescanned.
#[tauri::command]
async fn detect_check(app: AppHandle) -> CmdResult<bool> {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return Ok(false);
    }
    let r = blocking(move || {
        let m = manager()?;
        let stale = detect::cached_only(&m).map(|c| c.stale).unwrap_or(true);
        if !stale {
            return Ok(false);
        }
        let rep = detect::detect_fresh(&m);
        let _ = app.emit("detect-updated", &rep);
        Ok(true)
    })
    .await;
    CHECKING.store(false, Ordering::SeqCst);
    r
}

/// Convert legacy symlink adoptions to registry receipts (`dry_run` only reports).
#[tauri::command]
async fn migrate_adoptions(dry_run: bool) -> CmdResult<Vec<MigrationAction>> {
    blocking(move || {
        let m = manager()?;
        let acts = detect::migrate(&m, dry_run)?;
        if !dry_run {
            let _ = activate::reshim(&m, &activate::cli_path());
        }
        Ok(acts)
    })
    .await
}

#[derive(serde::Serialize)]
struct AdoptResult {
    version: String,
    set_global: bool,
}

#[tauri::command]
async fn adopt(item: Detected, name: Option<String>) -> CmdResult<AdoptResult> {
    blocking(move || {
        let m = manager()?;
        let r = detect::adopt_detected(&m, &item, name.as_deref().filter(|s| !s.is_empty()))?;
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(AdoptResult {
            version: r.0,
            set_global: r.1,
        })
    })
    .await
}

#[tauri::command]
async fn adopt_all() -> CmdResult<Vec<detect::AdoptOutcome>> {
    blocking(move || {
        let m = manager()?;
        let items = detect::detect_cached(&m).items;
        let r = detect::adopt_all(&m, &items);
        let _ = activate::reshim(&m, &activate::cli_path());
        Ok(r)
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
            installer_choice,
            set_install_via,
            uninstall,
            uninstall_plan,
            set_disabled,
            set_global,
            set_local,
            unset_local,
            detect_runtimes,
            detect_cached,
            detect_refresh,
            detect_check,
            migrate_adoptions,
            adopt,
            adopt_all,
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
