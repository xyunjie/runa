//! asdf compatible `.tool-versions` parsing and editing.

use std::path::Path;

use anyhow::{Context, Result};

/// asdf plugin names that map to runa tool names.
pub fn canonical_tool(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "nodejs" => "node".into(),
        "golang" => "go".into(),
        "jdk" | "openjdk" => "java".into(),
        "python3" => "python".into(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub tool: String,
    /// Versions in preference order (asdf allows fallbacks: `python 3.12.1 3.11.7`).
    pub versions: Vec<String>,
}

/// Parse `.tool-versions` text. Comments (`#`) and blank lines are ignored.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(tool) = parts.next() else { continue };
        let versions: Vec<String> = parts.map(|s| s.to_string()).collect();
        if versions.is_empty() {
            continue;
        }
        out.push(Entry {
            tool: canonical_tool(tool),
            versions,
        });
    }
    out
}

pub fn lookup(text: &str, tool: &str) -> Option<Vec<String>> {
    parse(text)
        .into_iter()
        .find(|e| e.tool == tool)
        .map(|e| e.versions)
}

/// Return new file content with `tool` set to `version`, preserving other lines,
/// comments and the original plugin alias (e.g. `nodejs`).
pub fn set(text: &str, tool: &str, version: Option<&str>) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in text.lines() {
        let content = line.split('#').next().unwrap_or("").trim();
        let first = content.split_whitespace().next().unwrap_or("");
        if !first.is_empty() && canonical_tool(first) == tool {
            if replaced {
                continue;
            }
            replaced = true;
            if let Some(v) = version {
                lines.push(format!("{first} {v}"));
            }
            continue;
        }
        lines.push(line.to_string());
    }
    if !replaced {
        if let Some(v) = version {
            lines.push(format!("{tool} {v}"));
        }
    }
    let mut s = lines.join("\n");
    if !s.is_empty() {
        s.push('\n');
    }
    s
}

pub fn set_in_file(path: &Path, tool: &str, version: Option<&str>) -> Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let new = set(&text, tool, version);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, new).with_context(|| format!("写入 {} 失败", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_aliases_and_fallbacks() {
        let t =
            "# comment\nnodejs 20.11.1 # inline\n\npython 3.12.1 3.11.7\ngolang   1.22.0\nbroken\n";
        let e = parse(t);
        assert_eq!(e.len(), 3);
        assert_eq!(
            e[0],
            Entry {
                tool: "node".into(),
                versions: vec!["20.11.1".into()]
            }
        );
        assert_eq!(e[1].versions, vec!["3.12.1", "3.11.7"]);
        assert_eq!(lookup(t, "go").unwrap(), vec!["1.22.0"]);
        assert!(lookup(t, "java").is_none());
    }

    #[test]
    fn set_preserves_other_lines() {
        let t = "# keep me\nnodejs 18.0.0\npython 3.12.1\n";
        let out = set(t, "node", Some("20.1.0"));
        assert_eq!(out, "# keep me\nnodejs 20.1.0\npython 3.12.1\n");
        let out = set(&out, "go", Some("1.22.0"));
        assert!(out.ends_with("go 1.22.0\n"));
        let out = set(&out, "python", None);
        assert!(!out.contains("python"));
        assert_eq!(set("", "java", Some("21")), "java 21\n");
    }
}
