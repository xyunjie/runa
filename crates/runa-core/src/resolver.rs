//! Version resolution.
//!
//! Priority (highest first):
//! 1. env var `RUNA_<TOOL>_VERSION` (e.g. `RUNA_NODE_VERSION=20`)
//! 2. walking up from the working directory, in each directory:
//!    `.tool-versions`, then the tool's legacy files
//!    (`.nvmrc`, `.node-version`, `package.json` engines.node, `.python-version`,
//!    `.java-version`, `.sdkmanrc`, `.go-version`, `go.mod`)
//! 3. global `~/.runa/.tool-versions`

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::toolversions;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "lowercase")]
pub enum Source {
    Env(String),
    File(PathBuf),
    Global(PathBuf),
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Env(k) => format!("环境变量 {k}"),
            Source::File(p) => p.display().to_string(),
            Source::Global(p) => format!("全局 {}", p.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Spec {
    pub tool: String,
    /// Candidate specs in preference order (usually one).
    pub specs: Vec<String>,
    pub source: Source,
}

pub fn env_key(tool: &str) -> String {
    let t: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("RUNA_{t}_VERSION")
}

/// Legacy (non `.tool-versions`) files understood for each tool, in priority order.
pub fn legacy_files(tool: &str) -> &'static [&'static str] {
    match tool {
        "node" => &[".nvmrc", ".node-version", "package.json"],
        "python" => &[".python-version"],
        "java" => &[".java-version", ".sdkmanrc"],
        "go" => &[".go-version", "go.mod"],
        "ruby" => &[".ruby-version"],
        "rust" => &["rust-toolchain.toml", "rust-toolchain"],
        "bun" => &[".bun-version"],
        _ => &[],
    }
}

/// Extract a version spec from a legacy file's content.
pub fn parse_legacy(file_name: &str, text: &str) -> Option<String> {
    match file_name {
        "package.json" => package_json_engine(text),
        "go.mod" => go_mod_version(text),
        ".sdkmanrc" => sdkman_java(text),
        ".ruby-version" => first_meaningful_line(text).map(|v| {
            // rbenv / chruby accept `ruby-3.3.0`
            v.strip_prefix("ruby-").map(str::to_string).unwrap_or(v)
        }),
        "rust-toolchain.toml" => rust_toolchain_toml(text),
        "rust-toolchain" if text.contains("[toolchain]") => rust_toolchain_toml(text),
        "rust-toolchain" => first_meaningful_line(text).map(|c| rust_channel(&c)),
        _ => first_meaningful_line(text),
    }
}

fn first_meaningful_line(text: &str) -> Option<String> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .find(|l| !l.is_empty())
        .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
}

fn package_json_engine(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let s = v.get("engines")?.get("node")?.as_str()?.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// `toolchain go1.22.3` wins over `go 1.22`.
fn go_mod_version(text: &str) -> Option<String> {
    let mut go = None;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix("toolchain ") {
            let t = rest.trim();
            if t != "default" {
                return Some(t.trim_start_matches("go").to_string());
            }
        } else if let Some(rest) = line.strip_prefix("go ") {
            // The go directive is a minimum language version: match on major.minor.
            let v = rest.trim();
            go = Some(v.split('.').take(2).collect::<Vec<_>>().join("."));
        }
    }
    go
}

/// `[toolchain] channel = "1.80.0"` (rustup's rust-toolchain.toml).
fn rust_toolchain_toml(text: &str) -> Option<String> {
    let mut in_table = false;
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') {
            in_table = l == "[toolchain]";
        } else if let Some(rest) = l.strip_prefix("channel").filter(|_| in_table) {
            let c = rest
                .trim_start()
                .strip_prefix('=')?
                .trim()
                .trim_matches(['"', '\'']);
            return (!c.is_empty()).then(|| rust_channel(c));
        }
    }
    None
}

/// rustup channel -> runa spec: `stable` / `1.80.0` / `1.80`; a host suffix
/// (`1.80.0-aarch64-apple-darwin`) or date (`nightly-2024-08-01`) is dropped.
fn rust_channel(c: &str) -> String {
    let c = c.trim();
    c.split('-').next().unwrap_or(c).to_string()
}

/// `.sdkmanrc`: `java=21.0.2-tem` -> `21.0.2`.
fn sdkman_java(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("java=") {
            let v = rest.trim();
            return Some(v.split('-').next().unwrap_or(v).to_string());
        }
    }
    None
}

/// Resolve the spec for `tool` starting at `cwd`.
pub fn find_spec(
    tool: &str,
    cwd: &Path,
    env: &dyn Fn(&str) -> Option<String>,
    global_file: &Path,
) -> Option<Spec> {
    let key = env_key(tool);
    if let Some(v) = env(&key).filter(|v| !v.trim().is_empty()) {
        return Some(Spec {
            tool: tool.into(),
            specs: vec![v.trim().to_string()],
            source: Source::Env(key),
        });
    }
    for dir in cwd.ancestors() {
        if let Some(spec) = spec_in_dir(tool, dir) {
            return Some(spec);
        }
    }
    let text = std::fs::read_to_string(global_file).ok()?;
    let specs = toolversions::lookup(&text, tool)?;
    Some(Spec {
        tool: tool.into(),
        specs,
        source: Source::Global(global_file.to_path_buf()),
    })
}

/// Look only at one directory (no walking).
pub fn spec_in_dir(tool: &str, dir: &Path) -> Option<Spec> {
    let tv = dir.join(".tool-versions");
    if let Ok(text) = std::fs::read_to_string(&tv) {
        if let Some(specs) = toolversions::lookup(&text, tool) {
            return Some(Spec {
                tool: tool.into(),
                specs,
                source: Source::File(tv),
            });
        }
    }
    for name in legacy_files(tool) {
        let p = dir.join(name);
        if let Ok(text) = std::fs::read_to_string(&p) {
            if let Some(v) = parse_legacy(name, &text) {
                return Some(Spec {
                    tool: tool.into(),
                    specs: vec![v],
                    source: Source::File(p),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn legacy_parsers() {
        assert_eq!(
            parse_legacy(".nvmrc", "v20.11.1\n").as_deref(),
            Some("v20.11.1")
        );
        assert_eq!(
            parse_legacy(".nvmrc", "# c\nlts/iron\n").as_deref(),
            Some("lts/iron")
        );
        assert_eq!(
            parse_legacy(
                "package.json",
                r#"{"name":"x","engines":{"node":">=18.0.0"}}"#
            )
            .as_deref(),
            Some(">=18.0.0")
        );
        assert_eq!(parse_legacy("package.json", r#"{"name":"x"}"#), None);
        assert_eq!(
            parse_legacy("go.mod", "module x\n\ngo 1.21\n").as_deref(),
            Some("1.21")
        );
        assert_eq!(
            parse_legacy("go.mod", "module x\n\ngo 1.22.4\n").as_deref(),
            Some("1.22")
        );
        assert_eq!(
            parse_legacy("go.mod", "module x\ngo 1.21\ntoolchain go1.22.3\n").as_deref(),
            Some("1.22.3")
        );
        assert_eq!(
            parse_legacy(".sdkmanrc", "java=21.0.2-tem\n").as_deref(),
            Some("21.0.2")
        );
        assert_eq!(
            parse_legacy(".python-version", "3.12.1\n").as_deref(),
            Some("3.12.1")
        );
    }

    #[test]
    fn priority_env_then_nearest_then_global() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let global = root.join("global.tool-versions");
        fs::write(&global, "node 18.0.0\npython 3.11\n").unwrap();
        let proj = root.join("proj");
        let sub = proj.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        fs::write(proj.join(".tool-versions"), "nodejs 20\n").unwrap();
        fs::write(proj.join("a/.nvmrc"), "22\n").unwrap();

        // nearest directory wins: a/.nvmrc beats proj/.tool-versions
        let s = find_spec("node", &sub, &no_env, &global).unwrap();
        assert_eq!(s.specs, vec!["22"]);
        assert_eq!(s.source, Source::File(proj.join("a/.nvmrc")));

        // in proj: .tool-versions
        let s = find_spec("node", &proj, &no_env, &global).unwrap();
        assert_eq!(s.specs, vec!["20"]);

        // falls back to global
        let s = find_spec("python", &sub, &no_env, &global).unwrap();
        assert_eq!(s.source, Source::Global(global.clone()));

        // env beats everything
        let env = |k: &str| (k == "RUNA_NODE_VERSION").then(|| "16".to_string());
        let s = find_spec("node", &sub, &env, &global).unwrap();
        assert_eq!(s.specs, vec!["16"]);
        assert_eq!(s.source, Source::Env("RUNA_NODE_VERSION".into()));

        assert!(find_spec("go", &sub, &no_env, &global).is_none());
    }

    #[test]
    fn tool_versions_beats_legacy_in_same_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        fs::write(d.join(".nvmrc"), "18\n").unwrap();
        fs::write(d.join(".tool-versions"), "node 20\n").unwrap();
        let s = spec_in_dir("node", d).unwrap();
        assert_eq!(s.specs, vec!["20"]);
        // .tool-versions without node falls through to legacy file
        fs::write(d.join(".tool-versions"), "python 3.12\n").unwrap();
        assert_eq!(spec_in_dir("node", d).unwrap().specs, vec!["18"]);
    }

    #[test]
    fn env_key_format() {
        assert_eq!(env_key("node"), "RUNA_NODE_VERSION");
        assert_eq!(env_key("my-tool"), "RUNA_MY_TOOL_VERSION");
    }
}
