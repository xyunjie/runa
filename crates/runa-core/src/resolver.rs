//! Version resolution.
//!
//! Priority (highest first):
//! 1. env var `RUNA_<TOOL>_VERSION` (e.g. `RUNA_NODE_VERSION=20`)
//! 2. walking up from the working directory, in each directory:
//!    `.tool-versions`, then the tool's legacy files
//!    (`.nvmrc`, `.node-version`, `package.json` engines.node, `.python-version`,
//!    `.java-version`, `.sdkmanrc`, `.go-version`, `go.mod`)
//! 3. build files, only when no explicit file was found in any ancestor:
//!    java: `pom.xml` (compiler release/source/target, parent poms),
//!    `build.gradle(.kts)` (toolchain / sourceCompatibility);
//!    python: `pyproject.toml` `requires-python`
//! 4. global `~/.runa/.tool-versions`

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
        "deno" => &[".dvmrc", ".deno-version"],
        _ => &[],
    }
}

/// Build files that imply a (lower-priority) requirement, in priority order.
pub fn build_files(tool: &str) -> &'static [&'static str] {
    match tool {
        "java" => &["pom.xml", "build.gradle.kts", "build.gradle"],
        "python" => &["pyproject.toml"],
        _ => &[],
    }
}

/// Extract a spec from a build file (may read parent files, hence path-based).
pub fn parse_build_file(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    match name {
        "pom.xml" => maven_java(path, 0, &[]),
        "build.gradle" | "build.gradle.kts" => gradle_java(&std::fs::read_to_string(path).ok()?),
        "pyproject.toml" => pyproject_python(&std::fs::read_to_string(path).ok()?),
        _ => None,
    }
}

/// `1.8` -> `8`, `17` -> `17`.
fn java_major(v: &str) -> Option<String> {
    let v = v.trim().trim_matches(['"', '\'']);
    let v = v.strip_prefix("1.").filter(|r| !r.is_empty()).unwrap_or(v);
    let major: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!major.is_empty() && major != "0").then_some(major)
}

fn strip_xml_comments(t: &str) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Inner text of the first `<tag>...</tag>` in `t`.
fn xml_block<'a>(t: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let i = t.find(&open)? + open.len();
    let j = t[i..].find(&close)?;
    Some(&t[i..i + j])
}

/// Direct `<k>v</k>` children (good enough for `<properties>`).
fn xml_pairs(t: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find('<') {
        let after = &rest[i + 1..];
        let Some(e) = after.find('>') else { break };
        let tag = &after[..e];
        if tag.starts_with('/') || tag.ends_with('/') || tag.contains(' ') {
            rest = &after[e + 1..];
            continue;
        }
        let close = format!("</{tag}>");
        let body = &after[e + 1..];
        match body.find(&close) {
            Some(j) => {
                out.push((tag.to_string(), body[..j].trim().to_string()));
                rest = &body[j + close.len()..];
            }
            None => rest = body,
        }
    }
    out
}

fn substitute(v: &str, props: &[(String, String)]) -> String {
    let mut v = v.to_string();
    for _ in 0..5 {
        let Some(i) = v.find("${") else { break };
        let Some(j) = v[i..].find('}') else { break };
        let key = &v[i + 2..i + j];
        let val = props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, x)| x.clone())
            .unwrap_or_default();
        v = format!("{}{}{}", &v[..i], val, &v[i + j + 1..]);
    }
    v
}

/// Java major from a Maven pom, following `<parent>` (relativePath, default `../pom.xml`).
fn maven_java(path: &Path, depth: usize, child_props: &[(String, String)]) -> Option<String> {
    if depth > 5 {
        return None;
    }
    let text = strip_xml_comments(&std::fs::read_to_string(path).ok()?);
    // child properties override parent ones: child entries come first
    let mut props = child_props.to_vec();
    if let Some(p) = xml_block(&text, "properties") {
        props.extend(xml_pairs(p));
    }
    let mut plugin_cfg = Vec::new();
    let mut search = text.as_str();
    while let Some(i) = search.find("<plugin>") {
        let rest = &search[i..];
        let end = rest.find("</plugin>").unwrap_or(rest.len());
        let block = &rest[..end];
        if block.contains("<artifactId>maven-compiler-plugin</artifactId>") {
            if let Some(cfg) = xml_block(block, "configuration") {
                plugin_cfg = xml_pairs(cfg);
            }
            break;
        }
        search = &rest[end..];
    }
    let get = |list: &[(String, String)], k: &str| {
        list.iter()
            .find(|(x, _)| x == k)
            .map(|(_, v)| substitute(v, &props))
            .and_then(|v| java_major(&v))
    };
    let found = get(&plugin_cfg, "release")
        .or_else(|| get(&props, "maven.compiler.release"))
        .or_else(|| get(&plugin_cfg, "target"))
        .or_else(|| get(&plugin_cfg, "source"))
        .or_else(|| get(&props, "maven.compiler.target"))
        .or_else(|| get(&props, "maven.compiler.source"))
        .or_else(|| get(&props, "java.version"));
    if found.is_some() {
        return found;
    }
    let parent = xml_block(&text, "parent")?;
    let rel = xml_block(parent, "relativePath")
        .map(str::trim)
        .unwrap_or("../pom.xml");
    if rel.is_empty() {
        return None;
    }
    let mut pp = path.parent()?.join(rel);
    if pp.is_dir() {
        pp = pp.join("pom.xml");
    }
    maven_java(&pp, depth + 1, &props)
}

/// `JavaLanguageVersion.of(17)`, `jvmToolchain(17)`, `sourceCompatibility = '17'`,
/// `JavaVersion.VERSION_1_8`.
fn gradle_java(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.split("//").next().unwrap_or("").trim())
        .collect();
    let after = |pat: &str| -> Option<String> {
        lines.iter().find_map(|l| {
            let i = l.find(pat)?;
            let r = &l[i + pat.len()..];
            let r = r.trim_start_matches([' ', '(', '=', '"', '\'']);
            let r = r.strip_prefix("JavaVersion.VERSION_").unwrap_or(r);
            let r = r.strip_prefix("JavaLanguageVersion.of(").unwrap_or(r);
            let r = r.trim_start_matches(['"', '\'']);
            let v: String = r
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_')
                .collect();
            java_major(&v.replace('_', "."))
        })
    };
    after("JavaLanguageVersion.of")
        .or_else(|| after("jvmToolchain"))
        .or_else(|| after("targetCompatibility"))
        .or_else(|| after("sourceCompatibility"))
}

/// `[project] requires-python = ">=3.10"` (any section, first hit).
fn pyproject_python(text: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if let Some(rest) = l.strip_prefix("requires-python") {
            let v = rest
                .trim_start()
                .strip_prefix('=')?
                .trim()
                .trim_matches(['"', '\'']);
            return (!v.trim().is_empty()).then(|| v.trim().to_string());
        }
    }
    None
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

/// Version files that hold just a version and can be rewritten in place by `runa use`.
pub fn is_plain_version_file(name: &str) -> bool {
    matches!(
        name,
        ".nvmrc"
            | ".node-version"
            | ".python-version"
            | ".java-version"
            | ".ruby-version"
            | ".bun-version"
            | ".go-version"
            | ".dvmrc"
            | ".deno-version"
    )
}

/// Nearest project file that sets `tool`, walking up from `cwd` but not into `stop`
/// (and its ancestors) — typically the user's home, whose `.tool-versions` is global-ish.
pub fn project_spec(tool: &str, cwd: &Path, stop: Option<&Path>) -> Option<Spec> {
    let dirs: Vec<&Path> = cwd
        .ancestors()
        .take_while(|d| stop.is_none_or(|s| !s.starts_with(d)))
        .collect();
    dirs.iter()
        .find_map(|d| explicit_in_dir(tool, d))
        .or_else(|| dirs.iter().find_map(|d| build_in_dir(tool, d)))
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
    if let Some(spec) = cwd
        .ancestors()
        .find_map(|d| explicit_in_dir(tool, d))
        .or_else(|| cwd.ancestors().find_map(|d| build_in_dir(tool, d)))
    {
        return Some(spec);
    }
    let text = std::fs::read_to_string(global_file).ok()?;
    let specs = toolversions::lookup(&text, tool)?;
    Some(Spec {
        tool: tool.into(),
        specs,
        source: Source::Global(global_file.to_path_buf()),
    })
}

/// Look only at one directory (no walking): explicit files, then build files.
pub fn spec_in_dir(tool: &str, dir: &Path) -> Option<Spec> {
    explicit_in_dir(tool, dir).or_else(|| build_in_dir(tool, dir))
}

/// Build files in one directory (lower priority than any explicit file).
pub fn build_in_dir(tool: &str, dir: &Path) -> Option<Spec> {
    build_files(tool).iter().find_map(|name| {
        let p = dir.join(name);
        let v = parse_build_file(&p)?;
        Some(Spec {
            tool: tool.into(),
            specs: vec![v],
            source: Source::File(p),
        })
    })
}

/// `.tool-versions` and legacy version files in one directory.
pub fn explicit_in_dir(tool: &str, dir: &Path) -> Option<Spec> {
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

    #[test]
    fn maven_detection() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        fs::write(
            root.join("pom.xml"),
            "<project><!-- <java.version>11</java.version> --><properties>\
             <jdk>1.8</jdk><java.version>${jdk}</java.version></properties></project>",
        )
        .unwrap();
        let api = root.join("api");
        fs::create_dir(&api).unwrap();
        fs::write(
            api.join("pom.xml"),
            "<project><parent><artifactId>p</artifactId></parent></project>",
        )
        .unwrap();
        assert_eq!(parse_build_file(&api.join("pom.xml")).as_deref(), Some("8"));
        let s = find_spec("java", &api, &no_env, &root.join("none")).unwrap();
        assert_eq!(s.specs, vec!["8"]);
        assert_eq!(s.source, Source::File(api.join("pom.xml")));
        // properties + plugin config (release wins)
        fs::write(
            api.join("pom.xml"),
            "<project><properties><maven.compiler.source>21</maven.compiler.source>\
             <maven.compiler.target>21</maven.compiler.target><r>17</r></properties><build><plugins>\
             <plugin><artifactId>other</artifactId><configuration><release>11</release></configuration></plugin>\
             <plugin><artifactId>maven-compiler-plugin</artifactId><configuration>\
             <annotationProcessorPaths><path><version>1</version></path></annotationProcessorPaths>\
             </configuration></plugin></plugins></build></project>",
        )
        .unwrap();
        assert_eq!(
            parse_build_file(&api.join("pom.xml")).as_deref(),
            Some("21")
        );
        fs::write(
            api.join("pom.xml"),
            "<project><properties><r>17</r></properties><build><plugins>\
             <plugin><artifactId>maven-compiler-plugin</artifactId><configuration>\
             <release>${r}</release></configuration></plugin></plugins></build></project>",
        )
        .unwrap();
        assert_eq!(
            parse_build_file(&api.join("pom.xml")).as_deref(),
            Some("17")
        );
        // an explicit file in an ancestor beats a build file nearer by
        fs::write(root.join(".java-version"), "11\n").unwrap();
        let s = find_spec("java", &api, &no_env, &root.join("none")).unwrap();
        assert_eq!(s.specs, vec!["11"]);
        assert_eq!(project_spec("java", &api, None).unwrap().specs, vec!["11"]);
    }

    #[test]
    fn gradle_and_pyproject_detection() {
        assert_eq!(
            gradle_java("java {\n toolchain { languageVersion = JavaLanguageVersion.of(17) }\n}")
                .as_deref(),
            Some("17")
        );
        assert_eq!(
            gradle_java("kotlin { jvmToolchain(21) }").as_deref(),
            Some("21")
        );
        assert_eq!(
            gradle_java("sourceCompatibility = '11'").as_deref(),
            Some("11")
        );
        assert_eq!(
            gradle_java("java { sourceCompatibility = JavaVersion.VERSION_1_8 }").as_deref(),
            Some("8")
        );
        assert_eq!(
            gradle_java("// jvmToolchain(9)\napply plugin: 'java'"),
            None
        );
        // multi-module: submodule without a version falls back to the root build file
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("build.gradle.kts"),
            "kotlin { jvmToolchain(17) }",
        )
        .unwrap();
        let m = d.path().join("app");
        fs::create_dir(&m).unwrap();
        fs::write(m.join("build.gradle.kts"), "plugins { java }").unwrap();
        let s = project_spec("java", &m, None).unwrap();
        assert_eq!(s.specs, vec!["17"]);
        assert_eq!(s.source, Source::File(d.path().join("build.gradle.kts")));
        assert_eq!(
            pyproject_python("[project]\nname = \"x\"\nrequires-python = \">=3.10,<3.13\"\n")
                .as_deref(),
            Some(">=3.10,<3.13")
        );
        fs::write(
            d.path().join("pyproject.toml"),
            "[project]\nrequires-python = \">=3.11\"\n",
        )
        .unwrap();
        assert_eq!(
            project_spec("python", &m, None).unwrap().specs,
            vec![">=3.11"]
        );
        fs::write(m.join(".python-version"), "3.12\n").unwrap();
        assert_eq!(
            project_spec("python", &m, None).unwrap().specs,
            vec!["3.12"]
        );
    }

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
    fn deno_and_project_spec_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let proj = home.join("p");
        let sub = proj.join("src");
        fs::create_dir_all(&sub).unwrap();
        fs::write(proj.join(".dvmrc"), "2.1.4\n").unwrap();
        assert_eq!(spec_in_dir("deno", &proj).unwrap().specs, vec!["2.1.4"]);
        fs::write(home.join(".tool-versions"), "node 18\n").unwrap();
        // home is a boundary: its .tool-versions is not a project file
        assert!(project_spec("node", &sub, Some(home)).is_none());
        assert!(project_spec("node", &sub, None).is_some());
        assert_eq!(
            project_spec("deno", &sub, Some(home)).unwrap().source,
            Source::File(proj.join(".dvmrc"))
        );
        assert!(is_plain_version_file(".nvmrc") && !is_plain_version_file("package.json"));
        // other parsers
        assert_eq!(
            parse_legacy("rust-toolchain.toml", "[toolchain]\nchannel = \"1.80.0\"\n").as_deref(),
            Some("1.80.0")
        );
        assert_eq!(
            parse_legacy("rust-toolchain", "stable\n").as_deref(),
            Some("stable")
        );
        assert_eq!(
            parse_legacy(".ruby-version", "ruby-3.3.0\n").as_deref(),
            Some("3.3.0")
        );
    }

    #[test]
    fn env_key_format() {
        assert_eq!(env_key("node"), "RUNA_NODE_VERSION");
        assert_eq!(env_key("my-tool"), "RUNA_MY_TOOL_VERSION");
    }
}
