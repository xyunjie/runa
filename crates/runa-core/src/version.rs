//! Version parsing, ordering and fuzzy matching.
//!
//! Supported specs: `latest`, `lts`, `system`, prefixes (`20`, `3.12`), exact versions,
//! npm-like ranges (`>=18`, `^20.1`, `~3.11.2`, `18.x`, `^18 || ^20`), asdf vendor
//! prefixes (`temurin-21.0.2+13`), and `v`/`go` prefixes.

use std::cmp::Ordering;

/// Numeric components at the start of a version: `21.0.12.1+1` -> [21, 0, 12, 1].
pub fn components(v: &str) -> Vec<u64> {
    let v = strip_prefixes(v);
    let mut out = Vec::new();
    for part in v.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            break;
        }
        out.push(digits.parse().unwrap_or(0));
        if digits.len() != part.len() {
            break;
        }
    }
    out
}

fn strip_prefixes(v: &str) -> &str {
    let v = v.trim();
    let v = v.strip_prefix('v').unwrap_or(v);
    v.strip_prefix("go").unwrap_or(v)
}

/// Pre-release builds (rc / beta / alpha / ea / dev) are excluded from fuzzy matches.
pub fn is_prerelease(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    ["rc", "beta", "alpha", "-ea", "dev", "pre", "nightly"]
        .iter()
        .any(|m| l.contains(m))
        || is_python_pre(&l)
}

fn is_python_pre(l: &str) -> bool {
    // Python style pre-releases such as 3.13.0a1 / 3.13.0b2.
    let comps = l.split('.').next_back().unwrap_or("");
    let mut seen_digit = false;
    for c in comps.chars() {
        if c.is_ascii_digit() {
            seen_digit = true;
        } else if seen_digit && (c == 'a' || c == 'b') {
            return true;
        } else {
            break;
        }
    }
    false
}

/// Compare two versions: numeric components first, then releases before pre-releases.
pub fn compare(a: &str, b: &str) -> Ordering {
    let (ca, cb) = (components(a), components(b));
    let n = ca.len().max(cb.len());
    for i in 0..n {
        let x = ca.get(i).copied().unwrap_or(0);
        let y = cb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    match (is_prerelease(a), is_prerelease(b)) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        _ => a.cmp(b),
    }
}

/// Sort newest first.
pub fn sort_desc(v: &mut [String]) {
    v.sort_by(|a, b| compare(b, a));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionReq {
    /// Newest stable version.
    Latest,
    /// Newest long-term-support version (plugins without LTS treat it as Latest).
    Lts,
    /// Use whatever the system provides, runa does not inject anything.
    System,
    /// Leading components must match (`20` matches `20.11.1`).
    Prefix(Vec<u64>),
    /// At least this version.
    AtLeast(Vec<u64>),
    /// `>=min` and below `max` (`<max`, or `<=max` when the flag is set):
    /// `>=18 <21`, `>=3.10,<3.13`, `~=3.10`.
    Range(Vec<u64>, Vec<u64>, bool),
    /// Exact string match (used for versions with build metadata).
    Exact(String),
}

fn cmp_comps(c: &[u64], other: &[u64]) -> std::cmp::Ordering {
    let n = c.len().max(other.len());
    for i in 0..n {
        let x = c.get(i).copied().unwrap_or(0);
        let y = other.get(i).copied().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// `>=a <b` / `>=a,<b` / `~=a.b` -> Range.
fn parse_range(s: &str) -> Option<VersionReq> {
    if let Some(rest) = s.strip_prefix("~=") {
        let c = components(rest);
        if c.len() < 2 {
            return None;
        }
        let mut max = c[..c.len() - 1].to_vec();
        *max.last_mut()? += 1;
        return Some(VersionReq::Range(c, max, false));
    }
    let toks: Vec<&str> = s
        .split([' ', ','])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let mut min = None;
    let mut max = None;
    for t in toks {
        if let Some(r) = t.strip_prefix(">=").or_else(|| t.strip_prefix('>')) {
            min = Some(components(r));
        } else if let Some(r) = t.strip_prefix("<=") {
            max = Some((components(r), true));
        } else {
            let r = t.strip_prefix('<')?;
            max = Some((components(r), false));
        }
    }
    let (max, incl) = max?;
    (!max.is_empty()).then(|| VersionReq::Range(min.unwrap_or_default(), max, incl))
}

impl VersionReq {
    pub fn parse(spec: &str) -> Self {
        let s = spec.trim();
        // "^18 || ^20": prefer the newest alternative.
        let s = s.rsplit("||").next().unwrap_or(s).trim();
        if let Some(r) = parse_range(s) {
            return r;
        }
        // ">=18": keep the lower bound only.
        let s = s.split([' ', ',']).next().unwrap_or("");
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "" | "latest" | "stable" | "*" | "current" | "node" => return Self::Latest,
            "lts" | "lts/*" => return Self::Lts,
            "system" => return Self::System,
            _ => {}
        }
        if lower.starts_with("lts/") {
            return Self::Lts;
        }
        let s = strip_vendor(s);
        if let Some(rest) = s.strip_prefix(">=").or_else(|| s.strip_prefix('>')) {
            return Self::AtLeast(components(rest));
        }
        if let Some(rest) = s.strip_prefix('^') {
            let c = components(rest);
            let keep = if c.first() == Some(&0) { 2 } else { 1 };
            return Self::Prefix(c.into_iter().take(keep).collect());
        }
        if let Some(rest) = s.strip_prefix('~') {
            let c = components(rest.trim_start_matches('>'));
            return Self::Prefix(c.into_iter().take(2).collect());
        }
        let s = s.trim_start_matches(['=', '<']);
        let core = strip_prefixes(s);
        // "18.x" / "3.12.*"
        if core.contains(".x") || core.contains(".*") || core.contains(".X") {
            return Self::Prefix(components(core));
        }
        let comps = components(core);
        if comps.is_empty() {
            return Self::Exact(core.to_string());
        }
        let numeric_only = core.chars().all(|c| c.is_ascii_digit() || c == '.');
        if numeric_only {
            Self::Prefix(comps)
        } else {
            Self::Exact(core.to_string())
        }
    }

    pub fn matches(&self, version: &str) -> bool {
        match self {
            Self::Latest | Self::Lts => !is_prerelease(version),
            Self::System => false,
            Self::Prefix(p) => {
                let c = components(version);
                !is_prerelease(version) && c.len() >= p.len() && c[..p.len()] == p[..]
            }
            Self::AtLeast(min) => {
                if is_prerelease(version) {
                    return false;
                }
                let c = components(version);
                let n = c.len().max(min.len());
                for i in 0..n {
                    let x = c.get(i).copied().unwrap_or(0);
                    let y = min.get(i).copied().unwrap_or(0);
                    if x != y {
                        return x > y;
                    }
                }
                true
            }
            Self::Range(min, max, incl) => {
                if is_prerelease(version) {
                    return false;
                }
                let c = components(version);
                let hi = cmp_comps(&c, max);
                cmp_comps(&c, min) != std::cmp::Ordering::Less
                    && (hi == std::cmp::Ordering::Less
                        || (*incl && hi == std::cmp::Ordering::Equal))
            }
            Self::Exact(s) => strip_prefixes(version) == s || version == s,
        }
    }

    /// Pick the newest candidate satisfying this requirement.
    pub fn best<'a, I: IntoIterator<Item = &'a String>>(&self, candidates: I) -> Option<String> {
        candidates
            .into_iter()
            .filter(|v| self.matches(v))
            .max_by(|a, b| compare(a, b))
            .cloned()
    }

    /// True when the spec names one precise version (no fuzzy part).
    pub fn is_fuzzy(&self) -> bool {
        !matches!(self, Self::Exact(_))
    }
}

/// asdf style vendor prefixes: `temurin-21.0.2+13.0.LTS`, `openjdk-21`, `python-3.12`.
fn strip_vendor(s: &str) -> &str {
    if let Some(idx) = s.find(|c: char| c.is_ascii_digit()) {
        let prefix = &s[..idx];
        if prefix.ends_with('-') && prefix.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
            return &s[idx..];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compare_orders_numerically() {
        assert_eq!(compare("1.10.0", "1.9.9"), Ordering::Greater);
        assert_eq!(compare("20.0.0", "20.0.0-rc1"), Ordering::Greater);
        assert_eq!(compare("3.13.0", "3.13.0a1"), Ordering::Greater);
        let mut l = v(&["1.2", "1.10", "1.9.1"]);
        sort_desc(&mut l);
        assert_eq!(l, v(&["1.10", "1.9.1", "1.2"]));
    }

    #[test]
    fn prefix_match_is_component_wise() {
        let r = VersionReq::parse("2");
        assert!(r.matches("2.0.1"));
        assert!(!r.matches("20.1.0"));
        let c = v(&["20.11.1", "20.9.0", "21.0.0", "18.19.0"]);
        assert_eq!(VersionReq::parse("20").best(&c).as_deref(), Some("20.11.1"));
        assert_eq!(
            VersionReq::parse("v20.9").best(&c).as_deref(),
            Some("20.9.0")
        );
        assert_eq!(
            VersionReq::parse("latest").best(&c).as_deref(),
            Some("21.0.0")
        );
    }

    #[test]
    fn npm_ranges() {
        let c = v(&["16.20.0", "18.19.0", "20.11.1", "22.1.0"]);
        assert_eq!(
            VersionReq::parse(">=18").best(&c).as_deref(),
            Some("22.1.0")
        );
        assert_eq!(
            VersionReq::parse("^18.2.0").best(&c).as_deref(),
            Some("18.19.0")
        );
        assert_eq!(
            VersionReq::parse("~20.11.0").best(&c).as_deref(),
            Some("20.11.1")
        );
        assert_eq!(
            VersionReq::parse("16.x").best(&c).as_deref(),
            Some("16.20.0")
        );
        assert_eq!(
            VersionReq::parse("^16 || ^20").best(&c).as_deref(),
            Some("20.11.1")
        );
        assert_eq!(
            VersionReq::parse(">=18 <21").best(&c).as_deref(),
            Some("20.11.1")
        );
    }

    #[test]
    fn special_specs() {
        assert_eq!(VersionReq::parse("lts/iron"), VersionReq::Lts);
        assert_eq!(VersionReq::parse("system"), VersionReq::System);
        assert_eq!(VersionReq::parse(""), VersionReq::Latest);
        assert_eq!(VersionReq::parse("go1.22"), VersionReq::Prefix(vec![1, 22]));
        assert_eq!(
            VersionReq::parse("temurin-21"),
            VersionReq::Prefix(vec![21])
        );
        assert!(VersionReq::parse("temurin-21.0.2+13").matches("21.0.2+13"));
        assert!(!VersionReq::parse("1.22").matches("1.22rc1"));
        assert!(VersionReq::parse("1.22rc1").matches("1.22rc1"));
    }

    #[test]
    fn java_versions() {
        let c = v(&["21.0.12.1", "17.0.9", "8.0.462"]);
        assert_eq!(
            VersionReq::parse("21").best(&c).as_deref(),
            Some("21.0.12.1")
        );
        assert_eq!(VersionReq::parse("8").best(&c).as_deref(), Some("8.0.462"));
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn ranges() {
        let r = VersionReq::parse(">=18 <21");
        assert!(
            r.matches("20.11.1")
                && r.matches("18.0.0")
                && !r.matches("21.0.0")
                && !r.matches("17.9.0")
        );
        let vs: Vec<String> = ["24.13.0", "22.22.2", "20.1.0"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(r.best(vs.iter()).as_deref(), Some("20.1.0"));
        let p = VersionReq::parse(">=3.10,<3.13");
        assert!(p.matches("3.12.2") && !p.matches("3.13.0") && !p.matches("3.9.1"));
        let c = VersionReq::parse("~=3.10");
        assert!(c.matches("3.12.2") && !c.matches("4.0.0") && !c.matches("3.9.0"));
        let le = VersionReq::parse(">=8, <=17");
        assert!(le.matches("17") && le.matches("17.0.0") && !le.matches("17.0.1"));
        assert_eq!(VersionReq::parse(">=18"), VersionReq::AtLeast(vec![18]));
        assert_eq!(
            VersionReq::parse(">=3.10,"),
            VersionReq::AtLeast(vec![3, 10])
        );
        assert_eq!(
            VersionReq::parse("^18 || ^20"),
            VersionReq::Prefix(vec![20])
        );
    }
}
