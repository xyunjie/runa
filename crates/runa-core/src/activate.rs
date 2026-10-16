//! Activation: shell hooks that rewrite PATH on every prompt, shims as a fallback, and
//! one-click shell profile setup.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::manager::{EnvPlan, Manager};

pub const SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];
const MARK_BEGIN: &str = "# >>> runa >>>";
const MARK_END: &str = "# <<< runa <<<";
const KEYS_VAR: &str = "_RUNA_ENV_KEYS";
const BASE_VAR: &str = "RUNA_PATH_BASE";
const UNSET: &str = "__runa_unset__";
const SHIM_MARK: &str = "runa-shim";

pub fn normalize_shell(s: &str) -> Result<String> {
    let s = s.trim().to_ascii_lowercase();
    let s = s
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&s)
        .trim_end_matches(".exe")
        .to_string();
    Ok(match s.as_str() {
        "bash" | "sh" => "bash".into(),
        "zsh" => "zsh".into(),
        "fish" => "fish".into(),
        "powershell" | "pwsh" | "ps" => "powershell".into(),
        other => bail!("不支持的 shell: {other}（支持 {}）", SHELLS.join(" / ")),
    })
}

/// Guess the user's shell from `$SHELL` (PowerShell on Windows).
pub fn default_shell() -> String {
    if let Ok(s) = std::env::var("SHELL") {
        if let Ok(n) = normalize_shell(&s) {
            return n;
        }
    }
    if cfg!(windows) {
        "powershell".into()
    } else {
        "bash".into()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Op {
    Set(String, String),
    Unset(String),
}

/// Compute the environment changes for a plan, given the shell's current environment.
///
/// PATH is always rebuilt from `RUNA_PATH_BASE`, so running the hook repeatedly never grows
/// PATH, and leaving a project removes its versions. Variables runa set earlier but no longer
/// needs (JAVA_HOME, GOROOT...) are restored to the user's original value or unset.
pub fn env_ops(plan: &EnvPlan, get: &dyn Fn(&str) -> Option<String>, path_sep: &str) -> Vec<Op> {
    let mut ops = Vec::new();
    let base = match get(BASE_VAR) {
        Some(b) => b,
        None => {
            let p = get("PATH").unwrap_or_default();
            ops.push(Op::Set(BASE_VAR.into(), p.clone()));
            p
        }
    };
    let mut parts: Vec<String> = plan.paths.iter().map(|p| p.display().to_string()).collect();
    if !base.is_empty() {
        parts.push(base);
    }
    ops.push(Op::Set("PATH".into(), parts.join(path_sep)));

    let new: BTreeMap<String, String> = plan.vars.iter().cloned().collect();
    let old: Vec<String> = get(KEYS_VAR)
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();
    for k in &old {
        if new.contains_key(k) {
            continue;
        }
        let orig_key = format!("_RUNA_ORIG_{k}");
        match get(&orig_key) {
            Some(v) if v != UNSET => ops.push(Op::Set(k.clone(), v)),
            _ => ops.push(Op::Unset(k.clone())),
        }
        ops.push(Op::Unset(orig_key));
    }
    for (k, v) in &new {
        if !old.contains(k) {
            let orig = get(k).unwrap_or_else(|| UNSET.into());
            ops.push(Op::Set(format!("_RUNA_ORIG_{k}"), orig));
        }
        ops.push(Op::Set(k.clone(), v.clone()));
    }
    let keys: Vec<&str> = new.keys().map(|s| s.as_str()).collect();
    if keys.is_empty() {
        ops.push(Op::Unset(KEYS_VAR.into()));
    } else {
        ops.push(Op::Set(KEYS_VAR.into(), keys.join(" ")));
    }
    ops
}

fn q_posix(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
fn q_fish(s: &str) -> String {
    format!("'{}'", s.replace('\\', r"\\").replace('\'', r"\'"))
}
fn q_ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn valid_key(k: &str) -> bool {
    !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn render_ops(shell: &str, ops: &[Op], path_sep: &str) -> Result<String> {
    let shell = normalize_shell(shell)?;
    let mut s = String::new();
    for op in ops {
        let k = match op {
            Op::Set(k, _) | Op::Unset(k) => k,
        };
        if !valid_key(k) {
            bail!("非法环境变量名: {k}");
        }
        if let Op::Set(_, v) = op {
            if v.contains(['\0', '\n', '\r']) {
                bail!("环境变量 {k} 的值包含非法字符");
            }
        }
        let line = match (shell.as_str(), op) {
            ("bash" | "zsh", Op::Set(k, v)) => format!("export {k}={}", q_posix(v)),
            ("bash" | "zsh", Op::Unset(k)) => format!("unset {k}"),
            ("fish", Op::Set(k, v)) if k == "PATH" => {
                let parts: Vec<String> = v
                    .split(path_sep)
                    .filter(|p| !p.is_empty())
                    .map(q_fish)
                    .collect();
                format!("set -gx PATH {}", parts.join(" "))
            }
            ("fish", Op::Set(k, v)) => format!("set -gx {k} {}", q_fish(v)),
            ("fish", Op::Unset(k)) => format!("set -e {k}"),
            (_, Op::Set(k, v)) => format!("$env:{k} = {}", q_ps(v)),
            (_, Op::Unset(k)) => format!("Remove-Item Env:{k} -ErrorAction SilentlyContinue"),
        };
        s.push_str(&line);
        s.push('\n');
    }
    Ok(s)
}

/// `runa env <shell>` output for `cwd`, based on the current process environment.
pub fn env_script(m: &Manager, shell: &str, cwd: &Path) -> Result<String> {
    let plan = m.env_plan(cwd, &[])?;
    let get = |k: &str| std::env::var(k).ok();
    let sep = if cfg!(windows) { ";" } else { ":" };
    let ops = env_ops(&plan, &get, sep);
    render_ops(shell, &ops, sep)
}

/// Script printed by `runa activate <shell>`: installs a prompt hook calling `runa env`.
pub fn hook_script(shell: &str, cli: &Path) -> Result<String> {
    let shell = normalize_shell(shell)?;
    let c = cli.display().to_string();
    Ok(match shell.as_str() {
        "bash" => format!(
            r#"_runa_hook() {{
  local previous_exit_status=$?
  eval "$({cli} env bash)"
  return $previous_exit_status
}}
if [[ ";${{PROMPT_COMMAND:-}};" != *";_runa_hook;"* ]]; then
  PROMPT_COMMAND="_runa_hook${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}"
fi
_runa_hook
"#,
            cli = q_posix(&c)
        ),
        "zsh" => format!(
            r#"_runa_hook() {{
  eval "$({cli} env zsh)"
}}
autoload -Uz add-zsh-hook
add-zsh-hook precmd _runa_hook
add-zsh-hook chpwd _runa_hook
_runa_hook
"#,
            cli = q_posix(&c)
        ),
        "fish" => format!(
            r#"function __runa_hook --on-event fish_prompt --on-variable PWD
    {cli} env fish | source
end
__runa_hook
"#,
            cli = q_fish(&c)
        ),
        _ => format!(
            r#"$global:__RunaCli = {cli}
if (-not $global:__RunaOrigPrompt) {{ $global:__RunaOrigPrompt = $function:prompt }}
function global:prompt {{
    (& $global:__RunaCli env powershell | Out-String) | Invoke-Expression
    & $global:__RunaOrigPrompt
}}
(& $global:__RunaCli env powershell | Out-String) | Invoke-Expression
"#,
            cli = q_ps(&c)
        ),
    })
}

/// Location of the CLI binary used by shims and hooks.
/// `RUNA_CLI` > sibling of the current executable > `runa` on PATH.
pub fn cli_path() -> PathBuf {
    if let Ok(p) = std::env::var("RUNA_CLI") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let exe_name = crate::platform::exe_name("runa");
    if let Ok(me) = std::env::current_exe() {
        if me
            .file_name()
            .map(|n| n == exe_name.as_str())
            .unwrap_or(false)
        {
            return me;
        }
        if let Some(dir) = me.parent() {
            let sib = dir.join(&exe_name);
            if sib.is_file() {
                return sib;
            }
        }
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for d in std::env::split_paths(&path) {
        let p = d.join(&exe_name);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe_name)
}

pub fn cli_available(cli: &Path) -> bool {
    cli.is_file()
}

/// Rebuild the shims directory. Returns the shim names.
pub fn reshim(m: &Manager, cli: &Path) -> Result<Vec<String>> {
    let dir = m.paths.shims();
    std::fs::create_dir_all(&dir)?;
    let names = m.all_commands();
    // drop stale shims (only files that we generated)
    for e in std::fs::read_dir(&dir)?.flatten() {
        let p = e.path();
        let ours = std::fs::read_to_string(&p)
            .map(|t| t.contains(SHIM_MARK))
            .unwrap_or(false);
        let stem = crate::platform::command_stem(&p).unwrap_or_default();
        if ours && !names.contains(&stem) {
            let _ = std::fs::remove_file(&p);
        }
    }
    for n in &names {
        write_shim(&dir, n, cli)?;
    }
    Ok(names)
}

fn write_shim(dir: &Path, name: &str, cli: &Path) -> Result<()> {
    if name.contains(['/', '\\', '"', '\'', '%', '$', '`']) {
        return Ok(());
    }
    if cfg!(windows) {
        let body = format!(
            "@echo off\r\nrem {SHIM_MARK}\r\n\"{}\" shim {name} %*\r\n",
            cli.display()
        );
        std::fs::write(dir.join(format!("{name}.cmd")), body)?;
    } else {
        let p = dir.join(name);
        let body = format!(
            "#!/bin/sh\n# {SHIM_MARK}\nexec {} shim {} \"$@\"\n",
            q_posix(&cli.display().to_string()),
            q_posix(name)
        );
        std::fs::write(&p, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

/// Profile file that `setup_profile` edits for a shell.
pub fn profile_path(shell: &str) -> Result<PathBuf> {
    let shell = normalize_shell(shell)?;
    let home = dirs::home_dir().context("无法确定用户主目录")?;
    Ok(match shell.as_str() {
        "bash" => {
            if cfg!(target_os = "macos") {
                home.join(".bash_profile")
            } else {
                home.join(".bashrc")
            }
        }
        "zsh" => std::env::var_os("ZDOTDIR")
            .map(PathBuf::from)
            .unwrap_or(home.clone())
            .join(".zshrc"),
        "fish" => dirs::config_dir()
            .unwrap_or(home.join(".config"))
            .join("fish")
            .join("conf.d")
            .join("runa.fish"),
        _ => {
            if cfg!(windows) {
                dirs::document_dir()
                    .unwrap_or(home.join("Documents"))
                    .join("PowerShell")
                    .join("Microsoft.PowerShell_profile.ps1")
            } else {
                home.join(".config")
                    .join("powershell")
                    .join("Microsoft.PowerShell_profile.ps1")
            }
        }
    })
}

/// Profile snippet: shims on PATH (for non-interactive use) + prompt hook.
pub fn profile_block(shell: &str, cli: &Path, shims: &Path) -> Result<String> {
    let shell = normalize_shell(shell)?;
    let c = cli.display().to_string();
    let s = shims.display().to_string();
    let body = match shell.as_str() {
        "bash" | "zsh" => format!(
            "export PATH={}:\"$PATH\"\neval \"$({} activate {shell})\"",
            q_posix(&s),
            q_posix(&c)
        ),
        "fish" => format!("fish_add_path -g {}\n{} activate fish | source", q_fish(&s), q_fish(&c)),
        _ => format!(
            "$env:PATH = {} + [IO.Path]::PathSeparator + $env:PATH\n(& {} activate powershell | Out-String) | Invoke-Expression",
            q_ps(&s),
            q_ps(&c)
        ),
    };
    Ok(format!("{MARK_BEGIN}\n{body}\n{MARK_END}\n"))
}

/// Insert or replace the runa block in `text`.
pub fn upsert_block(text: &str, block: &str) -> String {
    if let (Some(a), Some(b)) = (text.find(MARK_BEGIN), text.find(MARK_END)) {
        if a < b {
            let end = b + MARK_END.len();
            let end = if text[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            return format!("{}{}{}", &text[..a], block, &text[end..]);
        }
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(block);
    out
}

pub fn remove_block(text: &str) -> String {
    upsert_block(text, "").trim_end().to_string() + "\n"
}

pub fn profile_has_block(shell: &str) -> bool {
    profile_path(shell)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.contains(MARK_BEGIN))
        .unwrap_or(false)
}

/// Add (or refresh) activation in the shell profile. Returns the edited file.
pub fn setup_profile(m: &Manager, shell: &str, cli: &Path) -> Result<PathBuf> {
    let p = profile_path(shell)?;
    let block = profile_block(shell, cli, &m.paths.shims())?;
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if !text.is_empty() && !text.contains(MARK_BEGIN) {
        let _ = std::fs::write(p.with_extension("runa-backup"), &text);
    }
    std::fs::write(&p, upsert_block(&text, &block))
        .with_context(|| format!("写入 {} 失败", p.display()))?;
    Ok(p)
}

pub fn remove_profile(shell: &str) -> Result<Option<PathBuf>> {
    let p = profile_path(shell)?;
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Ok(None);
    };
    if !text.contains(MARK_BEGIN) {
        return Ok(None);
    }
    std::fs::write(&p, remove_block(&text))?;
    Ok(Some(p))
}

/// Windows: add the shims dir to the user PATH (registry, via PowerShell so that the
/// change is broadcast to new processes). No-op elsewhere.
pub fn add_shims_to_user_path(m: &Manager) -> Result<bool> {
    #[cfg(windows)]
    {
        let shims = m.paths.shims().display().to_string().replace('\'', "''");
        let script = format!(
            "$p=[Environment]::GetEnvironmentVariable('Path','User'); if (-not $p) {{ $p='' }}; \
             if (($p -split ';') -notcontains '{shims}') {{ [Environment]::SetEnvironmentVariable('Path', ('{shims};' + $p).TrimEnd(';'), 'User') }}"
        );
        let st = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .status()?;
        if !st.success() {
            bail!("更新用户 PATH 失败");
        }
        return Ok(true);
    }
    #[cfg(not(windows))]
    {
        let _ = m;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn plan(paths: &[&str], vars: &[(&str, &str)]) -> EnvPlan {
        EnvPlan {
            paths: paths.iter().map(PathBuf::from).collect(),
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            tools: vec![],
        }
    }

    #[test]
    fn first_activation_saves_base_and_originals() {
        let env: HashMap<&str, &str> = [("PATH", "/usr/bin"), ("JAVA_HOME", "/opt/old")].into();
        let get = |k: &str| env.get(k).map(|s| s.to_string());
        let ops = env_ops(
            &plan(&["/r/java/bin"], &[("JAVA_HOME", "/r/java")]),
            &get,
            ":",
        );
        assert!(ops.contains(&Op::Set("RUNA_PATH_BASE".into(), "/usr/bin".into())));
        assert!(ops.contains(&Op::Set("PATH".into(), "/r/java/bin:/usr/bin".into())));
        assert!(ops.contains(&Op::Set("_RUNA_ORIG_JAVA_HOME".into(), "/opt/old".into())));
        assert!(ops.contains(&Op::Set("JAVA_HOME".into(), "/r/java".into())));
        assert!(ops.contains(&Op::Set("_RUNA_ENV_KEYS".into(), "JAVA_HOME".into())));
    }

    #[test]
    fn leaving_project_restores_state() {
        let env: HashMap<&str, &str> = [
            ("PATH", "/r/java/bin:/usr/bin"),
            ("RUNA_PATH_BASE", "/usr/bin"),
            ("JAVA_HOME", "/r/java"),
            ("_RUNA_ORIG_JAVA_HOME", "/opt/old"),
            ("GOROOT", "/r/go"),
            ("_RUNA_ORIG_GOROOT", UNSET),
            ("_RUNA_ENV_KEYS", "GOROOT JAVA_HOME"),
        ]
        .into();
        let get = |k: &str| env.get(k).map(|s| s.to_string());
        let ops = env_ops(&plan(&[], &[]), &get, ":");
        assert!(ops.contains(&Op::Set("PATH".into(), "/usr/bin".into())));
        assert!(ops.contains(&Op::Set("JAVA_HOME".into(), "/opt/old".into())));
        assert!(ops.contains(&Op::Unset("GOROOT".into())));
        assert!(ops.contains(&Op::Unset("_RUNA_ENV_KEYS".into())));
        // idempotent PATH: base is never re-captured
        assert!(!ops
            .iter()
            .any(|o| matches!(o, Op::Set(k, _) if k == "RUNA_PATH_BASE")));
    }

    #[test]
    fn rendering_quotes_safely() {
        let ops = vec![
            Op::Set("PATH".into(), "/a b/it's:/usr/bin".into()),
            Op::Unset("X".into()),
        ];
        let bash = render_ops("bash", &ops, ":").unwrap();
        assert_eq!(bash, "export PATH='/a b/it'\\''s:/usr/bin'\nunset X\n");
        let fish = render_ops("fish", &ops, ":").unwrap();
        assert_eq!(fish, "set -gx PATH '/a b/it\\'s' '/usr/bin'\nset -e X\n");
        let ps = render_ops("pwsh", &ops, ":").unwrap();
        assert!(ps.starts_with("$env:PATH = '/a b/it''s:/usr/bin'"));
        assert!(render_ops("bash", &[Op::Set("A;B".into(), "x".into())], ":").is_err());
        assert!(render_ops("bash", &[Op::Set("A".into(), "x\ny".into())], ":").is_err());
        assert!(render_ops("cmd", &ops, ":").is_err());
    }

    #[test]
    fn profile_block_upsert_is_idempotent() {
        let block =
            profile_block("bash", Path::new("/opt/runa"), Path::new("/h/.runa/shims")).unwrap();
        let t1 = upsert_block("alias ll='ls -l'\n", &block);
        let t2 = upsert_block(&t1, &block);
        assert_eq!(t1, t2);
        assert_eq!(t1.matches(MARK_BEGIN).count(), 1);
        assert!(t1.starts_with("alias ll='ls -l'\n"));
        let removed = remove_block(&t1);
        assert!(!removed.contains(MARK_BEGIN));
        assert!(removed.contains("alias ll"));
    }

    #[test]
    fn hooks_reference_cli() {
        for sh in SHELLS {
            let s = hook_script(sh, Path::new("/opt/runa")).unwrap();
            assert!(s.contains("/opt/runa"), "{sh}");
        }
        assert_eq!(normalize_shell("/usr/bin/zsh").unwrap(), "zsh");
        assert_eq!(normalize_shell("pwsh.exe").unwrap(), "powershell");
    }
}
