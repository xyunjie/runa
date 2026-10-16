use anyhow::{bail, Result};

use super::{Artifact, Plugin};
use crate::net::Net;
use crate::platform::Target;
use crate::version;

const INDEX: &str = "https://cache.ruby-lang.org/pub/ruby/index.txt";

/// Ruby. ruby-lang.org only publishes source tarballs (there is no portable official
/// binary), so runa lists versions but installs through rbenv / rvm / mise / asdf.
pub struct Ruby;

/// Names in the ruby-lang.org index (`ruby-3.3.0<TAB>url ...`) -> `3.3.0`,
/// `3.4.0-preview1`.
pub fn parse_index(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|n| n.strip_prefix("ruby-"))
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()) && v.contains('.'))
        .map(str::to_string)
        .collect();
    version::sort_desc(&mut v);
    v.dedup();
    v
}

impl Plugin for Ruby {
    fn name(&self) -> &str {
        "ruby"
    }
    fn description(&self) -> String {
        "Ruby（官方只提供源码，推荐通过 rbenv / mise 安装）".into()
    }
    fn list_all(&self, net: &Net) -> Result<Vec<String>> {
        Ok(parse_index(&net.get_text(INDEX)?))
    }
    fn supports_download(&self, _t: &Target) -> bool {
        false
    }
    fn artifact(&self, _net: &Net, _version: &str, _t: &Target) -> Result<Artifact> {
        bail!(
            "Ruby 没有可移植的官方预编译包，runa 不直接下载；请通过 rbenv、rvm、mise 或 asdf 安装"
        )
    }
    fn detect_exes(&self) -> Vec<&'static str> {
        vec!["ruby"]
    }
    fn adopt_exes(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            "ruby", "gem", "irb", "erb", "rake", "rdoc", "ri", "bundle", "bundler",
        ])
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn index() {
        let t = "name\turl\tsha1\nruby-3.2.2\thttps://x/ruby-3.2.2.tar.gz\tab\nruby-3.3.0\tu.tar.gz\nruby-3.3.0\tu.zip\nruby-3.4.0-preview1\tu\nruby-0.49\tu\n";
        assert_eq!(
            super::parse_index(t),
            ["3.4.0-preview1", "3.3.0", "3.2.2", "0.49"]
        );
    }
}
