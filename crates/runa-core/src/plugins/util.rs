//! Helpers shared by the plugins added for Rust, Ruby, Bun and Deno.

/// First 64-hex-digit token in a checksum file (`<hash>  <file>`, PowerShell `Get-FileHash`
/// tables, ...).
pub fn first_sha256(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .find(|t| t.len() == 64)
        .map(|t| t.to_ascii_lowercase())
}

/// `x.y.z` versions from release-note headings: lines starting with `prefix` followed by
/// a version (`Version 1.80.0 (2024-07-25)`, `### 2.0.0 / 2024.10.09`).
pub fn versions_from_headings(text: &str, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix(prefix))
        .filter_map(|rest| {
            let tok = rest.split_whitespace().next()?.trim_start_matches('v');
            let ok = tok.split('.').count() >= 2
                && tok
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
            (ok && tok.starts_with(|c: char| c.is_ascii_digit())).then(|| tok.to_string())
        })
        .collect();
    crate::version::sort_desc(&mut v);
    v.dedup();
    v
}

/// Rust target triple for a runa target.
pub fn rust_triple(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("macos", "arm64") => "aarch64-apple-darwin",
        ("macos", "x64") => "x86_64-apple-darwin",
        ("linux", "x64") => "x86_64-unknown-linux-gnu",
        ("linux", "arm64") => "aarch64-unknown-linux-gnu",
        ("windows", "x64") => "x86_64-pc-windows-msvc",
        ("windows", "arm64") => "aarch64-pc-windows-msvc",
        ("windows", "x86") => "i686-pc-windows-msvc",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha_and_headings() {
        let h = "a".repeat(64);
        assert_eq!(first_sha256(&format!("{h}  deno.zip\n")), Some(h.clone()));
        assert_eq!(
            first_sha256(&format!("Algorithm Hash\nSHA256 {}\n", h.to_uppercase())),
            Some(h)
        );
        let md = "# Rust\nVersion 1.80.1 (2024-08-08)\n===\nVersion 1.80.0 (2024-07-25)\nVersion 1.9.0 (x)\n";
        assert_eq!(
            versions_from_headings(md, "Version "),
            ["1.80.1", "1.80.0", "1.9.0"]
        );
        let d = "### 2.0.0 / 2024.10.09\n### v1.46.3 / 2024.09.04\n### notes\n";
        assert_eq!(versions_from_headings(d, "### "), ["2.0.0", "1.46.3"]);
    }
}
