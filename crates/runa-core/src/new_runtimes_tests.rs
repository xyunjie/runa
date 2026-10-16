//! Rust, Ruby, Bun and Deno end to end with fake directories and fake managers.

// Some helpers are only used by Unix-only tests (fake shell-script managers).
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};

use crate::bootstrap;
use crate::detect::{self, Detected};
use crate::locations::{self, Env};
use crate::manager_admin;
use crate::managers::{self, Located, Probe};
use crate::resolver::{legacy_files, parse_legacy};
use crate::uninstall::{self, Mode, PlanKind};
use crate::Manager;

fn mgr(t: &Path) -> Manager {
    Manager::with_paths(crate::Paths::with_root(t.join("runa"))).unwrap()
}

fn env(home: &Path) -> Env {
    Env {
        home: home.to_path_buf(),
        ..Default::default()
    }
}

fn probe(home: &Path, path: Vec<PathBuf>) -> Probe {
    Probe {
        env: env(home),
        path,
        fallback: vec![],
    }
}

#[cfg(unix)]
fn script(p: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn registry_has_the_new_runtimes() {
    let t = tempfile::tempdir().unwrap();
    let m = mgr(t.path());
    for tool in ["rust", "ruby", "bun", "deno"] {
        assert!(m.plugin(tool).is_ok(), "{tool}");
    }
    let tgt = |os: &str, arch: &str| crate::Target {
        os: os.into(),
        arch: arch.into(),
    };
    let ruby = m.plugin("ruby").unwrap();
    assert!(!ruby.supports_download(&tgt("macos", "arm64")));
    let rust = m.plugin("rust").unwrap();
    assert!(!rust.supports_download(&tgt("linux", "x64")));
    assert!(!rust.supports_download(&tgt("windows", "x64")));
    assert_eq!(
        crate::plugins::bun::asset(&tgt("macos", "arm64")).unwrap(),
        "bun-darwin-aarch64"
    );
    assert_eq!(
        crate::plugins::bun::asset(&tgt("windows", "arm64")).unwrap(),
        "bun-windows-x64"
    );
    // rustc / ruby / bun / deno version output
    assert_eq!(
        rust.parse_version_output("rustc 1.80.0 (051478957 2024-07-21)")
            .as_deref(),
        Some("1.80.0")
    );
    assert_eq!(
        ruby.parse_version_output("ruby 3.3.0 (2023-12-25 revision 5124f9ac75) [arm64-darwin23]")
            .as_deref(),
        Some("3.3.0")
    );
    let deno = m.plugin("deno").unwrap();
    assert_eq!(
        deno.parse_version_output("deno 2.0.0 (stable, release, aarch64-apple-darwin)\nv8 12.9")
            .as_deref(),
        Some("2.0.0")
    );
    // runa's own Bun / Deno installs keep the binary at the top of the install dir
    let flat = t.path().join("flat");
    std::fs::create_dir_all(&flat).unwrap();
    assert_eq!(m.plugin("bun").unwrap().bin_dirs(&flat), vec![flat.clone()]);
    std::fs::create_dir_all(flat.join("bin")).unwrap();
    assert_eq!(deno.bin_dirs(&flat), vec![flat.join("bin")]);
}

#[test]
fn legacy_version_files() {
    assert_eq!(legacy_files("ruby"), [".ruby-version"]);
    assert_eq!(
        legacy_files("rust"),
        ["rust-toolchain.toml", "rust-toolchain"]
    );
    assert_eq!(legacy_files("bun"), [".bun-version"]);
    assert_eq!(
        parse_legacy(".ruby-version", "ruby-3.3.0\n").as_deref(),
        Some("3.3.0")
    );
    assert_eq!(
        parse_legacy(".ruby-version", "3.2.2").as_deref(),
        Some("3.2.2")
    );
    let toml = "# pinned\n[toolchain]\nchannel = \"1.80.0\"\ncomponents = [\"rustfmt\"]\n";
    assert_eq!(
        parse_legacy("rust-toolchain.toml", toml).as_deref(),
        Some("1.80.0")
    );
    assert_eq!(
        parse_legacy("rust-toolchain", "1.79.0-aarch64-apple-darwin\n").as_deref(),
        Some("1.79.0")
    );
    assert_eq!(
        parse_legacy("rust-toolchain", "[toolchain]\nchannel = 'stable'\n").as_deref(),
        Some("stable")
    );
    assert_eq!(
        parse_legacy(".bun-version", "1.1.30\n").as_deref(),
        Some("1.1.30")
    );
    for (alias, tool) in [
        ("rust", "rust"),
        ("ruby", "ruby"),
        ("bun", "bun"),
        ("deno", "deno"),
    ] {
        assert_eq!(crate::toolversions::canonical_tool(alias), tool);
    }
}

#[test]
fn stores_and_owners() {
    let h = PathBuf::from("/h");
    let e = env(&h);
    let dirs = |tool: &str| -> Vec<(PathBuf, &str)> {
        locations::stores(&e, tool)
            .into_iter()
            .map(|s| (s.dir, s.label))
            .collect()
    };
    let rust = dirs("rust");
    assert!(rust.contains(&(h.join(".rustup").join("toolchains"), "rustup")));
    assert!(rust.contains(&(h.join(".asdf").join("installs").join("rust"), "asdf")));
    let ruby = dirs("ruby");
    for (d, l) in [
        (h.join(".rbenv").join("versions"), "rbenv"),
        (h.join(".rvm").join("rubies"), "rvm"),
        (h.join(".rubies"), "chruby"),
        (h.join(".local/share/mise/installs/ruby"), "mise"),
    ] {
        assert!(ruby.contains(&(d.clone(), l)), "{d:?} {ruby:?}");
    }
    assert!(dirs("bun").contains(&(h.join("scoop").join("apps").join("bun"), "scoop")));
    assert!(dirs("deno").contains(&(h.join(".asdf").join("installs").join("deno"), "asdf")));
    assert!(locations::fixed_homes(&e, "bun").contains(&h.join(".bun")));
    assert!(locations::fixed_homes(&e, "deno").contains(&h.join(".deno")));

    let own = |p: &str| {
        let o = locations::owner(Path::new(p), &e);
        (o.provider, o.package, o.id)
    };
    assert_eq!(
        own("/h/.rustup/toolchains/stable-aarch64-apple-darwin"),
        (
            "rustup".into(),
            None,
            Some("stable-aarch64-apple-darwin".into())
        )
    );
    assert_eq!(
        own("/h/.rbenv/versions/3.3.0"),
        ("rbenv".into(), None, Some("3.3.0".into()))
    );
    assert_eq!(
        own("/h/.rvm/rubies/ruby-3.2.2"),
        ("rvm".into(), None, Some("ruby-3.2.2".into()))
    );
    assert_eq!(own("/h/.rubies/ruby-3.1.4").0, "chruby");
    assert_eq!(own("/other/.rbenv/versions/3.1.0").0, "rbenv");
    assert_eq!(
        own("/h/.local/share/mise/installs/bun/1.1.30"),
        ("mise".into(), Some("bun".into()), Some("1.1.30".into()))
    );
    assert_eq!(
        own("/opt/homebrew/Cellar/ruby/3.3.5"),
        ("homebrew".into(), Some("ruby".into()), Some("3.3.5".into()))
    );
    assert_eq!(own("/h/.bun").0, "manual");
    assert_eq!(own("/h/.deno").0, "manual");
    // macOS ruby and RubyInstaller
    assert_eq!(
        own("/System/Library/Frameworks/Ruby.framework/Versions/2.6/usr").0,
        "system"
    );
    assert_eq!(own(r"C:\Ruby33-x64").0, "installer");
    assert_eq!(
        own(r"C:\Users\a\scoop\apps\ruby\3.3.5-1").1.as_deref(),
        Some("ruby")
    );
    // rustup's proxy dir is never a toolchain
    assert!(locations::is_shim_dir(&h.join(".cargo").join("bin")));
    assert!(locations::is_shim_dir(&h.join(".cargo")));
    assert!(detect::is_os_bundled(Path::new(
        "/System/Library/Frameworks/Ruby.framework/Versions/2.6/usr"
    )));
}

#[cfg(unix)]
#[test]
fn detection_in_fake_stores_and_brew() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let mk = |d: &Path, exe: &str| script(&d.join("bin").join(exe), "#!/bin/sh\n");
    mk(&h.join(".rbenv/versions/3.3.0"), "ruby");
    mk(&h.join(".rvm/rubies/ruby-3.2.2"), "ruby");
    std::fs::create_dir_all(h.join(".rvm/rubies/default")).unwrap(); // not ruby-*
    mk(
        &h.join(".rustup/toolchains/1.80.0-x86_64-unknown-linux-gnu"),
        "rustc",
    );
    let cellar = h.join("Cellar");
    mk(&cellar.join("deno/2.0.0"), "deno");
    mk(&cellar.join("ruby@3.2/3.2.5"), "ruby");
    mk(&cellar.join("rubyx/1.0"), "ruby"); // not a ruby formula
    let e = env(h);
    let found = |tool: &str| -> Vec<(PathBuf, String)> {
        detect::store_bin_dirs(&e, tool, std::slice::from_ref(&cellar))
            .into_iter()
            .filter(|(d, _)| d.is_dir())
            .collect()
    };
    let ruby = found("ruby");
    assert!(ruby.contains(&(h.join(".rbenv/versions/3.3.0/bin"), "rbenv".into())));
    assert!(ruby.contains(&(h.join(".rvm/rubies/ruby-3.2.2/bin"), "rvm".into())));
    assert!(ruby.contains(&(cellar.join("ruby@3.2/3.2.5/bin"), "homebrew".into())));
    assert!(!ruby
        .iter()
        .any(|(d, _)| d.starts_with(cellar.join("rubyx"))));
    assert!(!ruby.iter().any(|(d, _)| d.ends_with("default/bin")));
    assert_eq!(
        found("rust"),
        vec![(
            h.join(".rustup/toolchains/1.80.0-x86_64-unknown-linux-gnu/bin"),
            "rustup".into()
        )]
    );
    assert_eq!(
        found("deno"),
        vec![(cellar.join("deno/2.0.0/bin"), "homebrew".into())]
    );

    // the runtime reports its own home (through rustup proxies / rbenv shims)
    let real = crate::platform::canonicalize(h);
    let tc = real.join(".rustup/toolchains/1.80.0-x86_64-unknown-linux-gnu");
    let proxy = real.join(".cargo/bin/rustc");
    script(&proxy, &format!("#!/bin/sh\necho {}\n", tc.display()));
    assert_eq!(detect::query_home("rust", &proxy), Some(tc));
    let rb = real.join(".rbenv/versions/3.3.0");
    let shim = real.join(".rbenv/shims/ruby");
    script(&shim, &format!("#!/bin/sh\nprintf %s {}\n", rb.display()));
    assert_eq!(detect::query_home("ruby", &shim), Some(rb));
    // rustup proxy without a toolchain: ~/.cargo is never taken for a Rust home
    script(
        &proxy,
        "#!/bin/sh\necho 'error: no default toolchain' >&2\nexit 1\n",
    );
    assert_eq!(detect::query_home("rust", &proxy), None);
    assert_eq!(detect::guess_home(&proxy, "rustc"), None);
}

#[test]
fn installer_preferences() {
    use managers::candidates_for;
    assert_eq!(candidates_for("rust", false), ["rustup", "mise", "asdf"]);
    assert_eq!(candidates_for("rust", true), ["rustup", "mise"]);
    assert_eq!(
        candidates_for("ruby", false),
        ["rbenv", "rvm", "mise", "asdf"]
    );
    assert_eq!(candidates_for("ruby", true), ["mise"]);
    assert_eq!(candidates_for("bun", false), ["mise", "asdf"]);
    assert_eq!(candidates_for("deno", true), ["mise"]);

    let t = tempfile::tempdir().unwrap();
    let p0 = probe(t.path(), vec![]);
    // nothing installed: bun / deno fall back to runa's download, ruby cannot
    assert_eq!(
        managers::choose_for(&p0, "bun", "auto", false).unwrap().0,
        "runa"
    );
    let e = managers::choose_for(&p0, "ruby", "auto", false).unwrap_err();
    assert!(e.to_string().contains("rbenv / rvm / mise / asdf"), "{e}");
    assert!(managers::choose_for(&p0, "ruby", "runa", false).is_err());
    assert!(managers::choose_for(&p0, "rust", "runa", true).is_err());
    assert!(managers::choose_for(&p0, "rust", "runa", false).is_err());
    assert!(managers::choose_for(&p0, "rust", "auto", false).is_err());
    assert!(crate::plugins::deno::has_checksums("2.1.0"));
    assert!(crate::plugins::deno::has_checksums("2.5.6"));
    assert!(!crate::plugins::deno::has_checksums("2.0.6"));
    assert!(!crate::plugins::deno::has_checksums("1.46.3"));
    let c = managers::installer_choice(&p0, "ruby", "auto");
    let runa = c.options.iter().find(|o| o.via == "runa").unwrap();
    assert!(!runa.available);
    assert!(c.chosen.contains("不可用"));
    assert!(managers::VIAS.contains(&"rustup") && managers::VIAS.contains(&"rbenv"));
}

#[test]
fn manager_commands_and_homes() {
    let t = tempfile::tempdir().unwrap();
    let store = t.path().join(".rustup");
    let tc = store.join("toolchains").join("1.80.0-aarch64-apple-darwin");
    std::fs::create_dir_all(&tc).unwrap();
    std::fs::create_dir_all(store.join("toolchains").join("1.8.0-x")).unwrap();
    let loc = Located {
        entry: t.path().join(".cargo/bin/rustup"),
        store: store.clone(),
    };
    let c = managers::install_command("rustup", "rust", "1.80.0", &loc).unwrap();
    assert_eq!(
        c.display,
        "rustup toolchain install 1.80.0 --profile default"
    );
    assert_eq!(
        c.env,
        vec![("RUSTUP_HOME".into(), store.display().to_string())]
    );
    assert_eq!(
        managers::expected_homes("rustup", "rust", "1.80.0", &store),
        vec![tc]
    );
    let rb = Path::new("/h/.rbenv");
    let loc = Located {
        entry: rb.join("bin/rbenv"),
        store: rb.into(),
    };
    let c = managers::install_command("rbenv", "ruby", "3.3.0", &loc).unwrap();
    assert_eq!(c.display, "rbenv install -s 3.3.0");
    assert_eq!(
        managers::expected_homes("rbenv", "ruby", "3.3.0", rb),
        vec![rb.join("versions/3.3.0")]
    );
    let c = managers::install_command("rvm", "ruby", "3.3.0", &loc).unwrap();
    assert_eq!(c.program, PathBuf::from("bash"));
    assert!(c.args[1].contains(r#"rvm install "$2""#));
    assert_eq!(
        managers::expected_homes("rvm", "ruby", "3.3.0", Path::new("/h/.rvm")),
        vec![PathBuf::from("/h/.rvm/rubies/ruby-3.3.0")]
    );
    let c = managers::install_command("mise", "deno", "2.0.0", &loc).unwrap();
    assert_eq!(c.args, ["install", "deno@2.0.0"]);
    let c = managers::install_command("asdf", "ruby", "3.3.0", &loc).unwrap();
    assert_eq!(c.args, ["install", "ruby", "3.3.0"]);
    assert!(managers::install_command("rbenv", "ruby", "3.3;rm", &loc).is_err());
}

#[cfg(unix)]
#[test]
fn bootstrap_methods_for_rustup_and_rbenv() {
    let t = tempfile::tempdir().unwrap();
    let bin = t.path().join("bin");
    for b in ["brew", "bash", "curl", "git", "winget", "scoop"] {
        script(&bin.join(b), "#!/bin/sh\n");
    }
    let p = probe(t.path(), vec![bin]);
    let unix = bootstrap::methods_for(&p, "rustup", false);
    assert_eq!(unix.len(), 1);
    assert_eq!(unix[0].method, "script");
    assert!(unix[0].display.contains("https://sh.rustup.rs"));
    assert!(unix[0].display.contains("--no-modify-path"));
    let win = bootstrap::methods_for(&p, "rustup", true);
    assert_eq!(win[0].method, "winget");
    assert!(win[0].args.contains(&"Rustlang.Rustup".to_string()));
    assert_eq!(win[1].display, "scoop install rustup");
    let rb = bootstrap::methods_for(&p, "rbenv", false);
    assert_eq!(rb[0].display, "brew install rbenv ruby-build");
    assert_eq!(rb[1].method, "git");
    assert!(rb[1].display.contains("plugins/ruby-build"));
    assert!(bootstrap::methods_for(&p, "rbenv", true).is_empty());
    assert!(bootstrap::methods_for(&p, "rvm", false).is_empty());
    assert!(bootstrap::plan(&p, "rvm").is_err());
    assert_eq!(bootstrap::tools_of("rustup"), ["rust"]);
    assert_eq!(bootstrap::tools_of("rvm"), ["ruby"]);
    assert!(bootstrap::tools_of("mise").contains(&"deno"));
}

fn adopted(tool: &str, v: &str, home: &Path, bin: &Path) -> Detected {
    let (provider, formula) = detect::classify(home);
    Detected {
        tool: tool.into(),
        version: v.into(),
        exe: bin.join(if tool == "rust" { "rustc" } else { tool }),
        found_in: bin.into(),
        home: home.into(),
        via: "PATH".into(),
        provider,
        formula,
        adopted: false,
        adopted_as: None,
        disabled: false,
        system_default: false,
        runa_global: false,
        view_only: false,
    }
}

/// Uninstall through the owning tool: rustup toolchain uninstall, rbenv, rvm, mise, brew.
#[cfg(unix)]
#[test]
fn uninstall_plans_through_owner() {
    let t = tempfile::tempdir().unwrap();
    let h = crate::platform::canonicalize(t.path());
    let m = mgr(&h);
    let mk = |d: &Path, exe: &str| {
        script(&d.join("bin").join(exe), "#!/bin/sh\n");
        d.to_path_buf()
    };
    let cases = [
        (
            "rust",
            "1.80.0",
            mk(
                &h.join(".rustup/toolchains/1.80.0-aarch64-apple-darwin"),
                "rustc",
            ),
            "rustup toolchain uninstall 1.80.0-aarch64-apple-darwin",
        ),
        (
            "ruby",
            "3.3.0",
            mk(&h.join(".rbenv/versions/3.3.0"), "ruby"),
            "rbenv uninstall -f 3.3.0",
        ),
        (
            "ruby",
            "3.2.2",
            mk(&h.join(".rvm/rubies/ruby-3.2.2"), "ruby"),
            "rvm remove ruby-3.2.2",
        ),
        (
            "bun",
            "1.1.30",
            mk(&h.join("mise/installs/bun/1.1.30"), "bun"),
            "mise uninstall bun@1.1.30",
        ),
        (
            "deno",
            "2.0.0",
            mk(&h.join(".asdf/installs/deno/2.0.0"), "deno"),
            "asdf uninstall deno 2.0.0",
        ),
    ];
    for (tool, v, home, cmd) in &cases {
        detect::adopt(&m, &adopted(tool, v, home, &home.join("bin")), None).unwrap();
        let p = uninstall::plan(&m, tool, v, Mode::Purge).unwrap();
        assert_eq!(p.kind, PlanKind::ProviderCommand, "{tool} {v}");
        assert!(
            p.command_line.as_deref().unwrap_or("").contains(cmd),
            "{tool} {v}: {:?}",
            p.command_line
        );
    }
    // chruby rubies and ~/.bun are plain directories
    let chr = mk(&h.join(".rubies/ruby-3.1.4"), "ruby");
    detect::adopt(&m, &adopted("ruby", "3.1.4", &chr, &chr.join("bin")), None).unwrap();
    let p = uninstall::plan(&m, "ruby", "3.1.4", Mode::Purge).unwrap();
    assert_eq!(p.kind, PlanKind::RemoveDir);
    assert_eq!(p.paths[0], chr);
}

/// A fake rustup installs a toolchain into RUSTUP_HOME; runa registers it as a rustup
/// toolchain, resolves it from rust-toolchain.toml and purges it through rustup again.
#[cfg(unix)]
#[test]
fn fake_rustup_install_resolve_and_uninstall() {
    let t = tempfile::tempdir().unwrap();
    let h = crate::platform::canonicalize(t.path());
    let rustup_home = h.join(".rustup");
    std::fs::create_dir_all(rustup_home.join("toolchains")).unwrap();
    script(
        &h.join(".cargo/bin/rustup"),
        r#"#!/bin/sh
case "$1 $2" in
"toolchain install")
  d="$RUSTUP_HOME/toolchains/$3-x86_64-unknown-linux-gnu"; echo "installing $3"
  mkdir -p "$d/bin"
  for e in rustc cargo; do printf '#!/bin/sh\necho %s %s\n' "$e" "$3" > "$d/bin/$e"; chmod +x "$d/bin/$e"; done;;
"toolchain uninstall") rm -rf "$RUSTUP_HOME/toolchains/$3";;
esac
"#,
    );
    let m = mgr(&h);
    let p = Probe {
        env: Env {
            home: h.clone(),
            ..Default::default()
        },
        path: vec![],
        fallback: vec![],
    };
    let loc = p.locate("rustup", "rust").expect("fake rustup");
    assert_eq!(loc.store, rustup_home);
    let v =
        managers::install_with_manager(&m, "rustup", &loc, "rust", "1.80.0", &mut |_| {}).unwrap();
    assert_eq!(v, "1.80.0");
    let r = m.receipt("rust", "1.80.0").unwrap();
    assert_eq!(r.provider.as_deref(), Some("rustup"));
    // shims / resolution
    assert!(m.all_commands().contains(&"cargo".to_string()));
    let proj = h.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.80.0\"\n",
    )
    .unwrap();
    assert_eq!(
        m.resolve("rust", &proj).unwrap().version.as_deref(),
        Some("1.80.0")
    );
    m.set_global("rust", "1.80.0").unwrap();
    assert_eq!(m.global_version("rust").as_deref(), Some("1.80.0"));
    // disable keeps it registered but unused
    m.set_disabled("rust", "1.80.0", true, &proj).unwrap();
    assert!(m.enabled_versions("rust").is_empty());
    m.set_disabled("rust", "1.80.0", false, &proj).unwrap();
    let plan = uninstall::plan(&m, "rust", "1.80.0", Mode::Purge).unwrap();
    assert_eq!(
        plan.command_line.as_deref(),
        Some("rustup toolchain uninstall 1.80.0-x86_64-unknown-linux-gnu")
    );
    uninstall::execute(&m, &plan).unwrap();
    assert!(m.installed("rust").is_empty());
    assert!(!rustup_home
        .join("toolchains/1.80.0-x86_64-unknown-linux-gnu")
        .exists());
}

#[cfg(unix)]
#[test]
fn fake_rbenv_install_and_ruby_version_file() {
    let t = tempfile::tempdir().unwrap();
    let h = crate::platform::canonicalize(t.path());
    let root = h.join(".rbenv");
    script(
        &root.join("bin/rbenv"),
        r#"#!/bin/sh
case "$1" in
install) v="$3"; mkdir -p "$RBENV_ROOT/versions/$v/bin"
  printf '#!/bin/sh\necho ruby %s\n' "$v" > "$RBENV_ROOT/versions/$v/bin/ruby"; chmod +x "$RBENV_ROOT/versions/$v/bin/ruby";;
uninstall) rm -rf "$RBENV_ROOT/versions/$3";;
esac
"#,
    );
    let m = mgr(&h);
    let p = probe(&h, vec![]);
    assert!(bootstrap::is_installed(&p, "rbenv"));
    let (via, loc) = managers::choose_for(&p, "ruby", "auto", false).unwrap();
    assert_eq!(via, "rbenv");
    let v =
        managers::install_with_manager(&m, "rbenv", &loc.unwrap(), "ruby", "3.3.0", &mut |_| {})
            .unwrap();
    assert_eq!(
        m.receipt("ruby", &v).unwrap().provider.as_deref(),
        Some("rbenv")
    );
    let proj = h.join("app");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join(".ruby-version"), "ruby-3.3.0\n").unwrap();
    assert_eq!(
        m.resolve("ruby", &proj).unwrap().version.as_deref(),
        Some("3.3.0")
    );
    let info = manager_admin::info_one(&m, &p, "rbenv");
    assert!(info.installed);
    assert_eq!(info.runtimes.len(), 1);
    assert_eq!(
        manager_admin::data_runtimes("rbenv", &root),
        vec![root.join("versions/3.3.0")]
    );
    // no .git: unknown origin, runa refuses to uninstall the manager itself
    let pl = manager_admin::plan(&m, &p, "rbenv", "uninstall").unwrap();
    assert_eq!(pl.kind, manager_admin::PlanKind::Refuse);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let pl = manager_admin::plan_with(&m, &p, "rbenv", "uninstall", true).unwrap();
    assert_eq!(pl.kind, manager_admin::PlanKind::Run);
    assert!(pl.remove_dirs.contains(&root.join("bin")));
    assert!(!pl.remove_dirs.contains(&root.join("versions")));
    let up = manager_admin::plan(&m, &p, "rbenv", "upgrade").unwrap();
    assert!(up.command_line.unwrap().ends_with("pull --ff-only"));
}

/// rustup installed by its script: uninstall runs `rustup self uninstall`; keeping the
/// toolchains removes only rustup and its proxies.
#[cfg(unix)]
#[test]
fn rustup_manager_uninstall_plans() {
    let t = tempfile::tempdir().unwrap();
    let h = crate::platform::canonicalize(t.path());
    let cargo_bin = h.join(".cargo/bin");
    let body = "#!/bin/sh\necho 'rustup 1.27.1 (54dd3d00f 2024-04-24)'\n";
    script(&cargo_bin.join("rustup"), body);
    script(&cargo_bin.join("cargo"), body); // proxy: same binary
    script(
        &cargo_bin.join("ripgrep"),
        "#!/bin/sh\n# a cargo install tool\n",
    );
    let tc = h.join(".rustup/toolchains/stable-x86_64-unknown-linux-gnu");
    std::fs::create_dir_all(tc.join("bin")).unwrap();
    std::fs::write(h.join(".rustup/settings.toml"), "").unwrap();
    std::fs::write(h.join(".zshrc"), ". \"$HOME/.cargo/env\"\nalias x=y\n").unwrap();
    let m = mgr(&h);
    let p = probe(&h, vec![]);
    let info = manager_admin::info_one(&m, &p, "rustup");
    assert!(info.installed);
    assert_eq!(info.origin, "script");
    assert_eq!(info.version.as_deref(), Some("1.27.1"));
    let all = manager_admin::plan(&m, &p, "rustup", "uninstall").unwrap();
    assert_eq!(
        all.command_line.as_deref(),
        Some("rustup self uninstall -y")
    );
    assert!(all.remove_dirs.contains(&h.join(".rustup")));
    assert!(all.remove_dirs.contains(&h.join(".cargo")));
    assert_eq!(all.data_runtimes, vec![tc.clone()]);
    assert_eq!(all.rc_edits.len(), 1);
    let keep = manager_admin::plan_with(&m, &p, "rustup", "uninstall", true).unwrap();
    assert!(keep.keep_data && keep.command.is_none());
    assert!(keep.remove_dirs.contains(&cargo_bin.join("rustup")));
    assert!(keep.remove_dirs.contains(&cargo_bin.join("cargo")));
    assert!(keep.remove_dirs.contains(&h.join(".rustup/settings.toml")));
    assert!(!keep.remove_dirs.contains(&cargo_bin.join("ripgrep")));
    assert!(!keep.remove_dirs.iter().any(|d| d.ends_with("toolchains")));
    assert!(
        keep.rc_edits.is_empty(),
        "~/.cargo/env stays for cargo tools"
    );
    let up = manager_admin::plan(&m, &p, "rustup", "upgrade").unwrap();
    assert_eq!(up.command_line.as_deref(), Some("rustup self update"));
}

/// macOS /usr/bin/ruby: the framework prefix under /System is never taken as the home,
/// so detection falls back to the shared /usr prefix (view-only system runtime).
#[cfg(unix)]
#[test]
fn macos_system_ruby_is_view_only() {
    let t = tempfile::tempdir().unwrap();
    let stub = t.path().join("ruby");
    script(
        &stub,
        "#!/bin/sh\nprintf %s /System/Library/Frameworks/Ruby.framework/Versions/2.6/usr\n",
    );
    assert_eq!(detect::query_home("ruby", &stub), None);
    assert_eq!(
        detect::system_prefix_of(Path::new("/usr/bin/ruby")),
        Some(PathBuf::from("/usr"))
    );
    assert_eq!(detect::guess_home(Path::new("/usr/bin/ruby"), "ruby"), None);
}

/// Bun gets its `bunx` alias.
#[cfg(unix)]
#[test]
fn post_extract_layouts() {
    use crate::plugins::Plugin;
    let t = tempfile::tempdir().unwrap();
    let b = t.path().join("bun");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("bun"), "").unwrap();
    crate::plugins::bun::Bun.post_extract(&b).unwrap();
    assert_eq!(
        std::fs::read_link(b.join("bunx")).unwrap(),
        PathBuf::from("bun")
    );
}

#[test]
fn set_project_version_targets_the_right_file() {
    let t = tempfile::tempdir().unwrap();
    let m = Manager::with_paths(crate::Paths::with_root(t.path().join("runa"))).unwrap();
    let home = t.path().join("home");
    let proj = home.join("proj");
    let sub = proj.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let rd = |p: &std::path::Path| std::fs::read_to_string(p).unwrap();
    // nothing yet: .tool-versions in the directory itself
    let f = m
        .set_project_version(&sub, "python", "3.12.1", Some(&home))
        .unwrap();
    assert_eq!(f, sub.join(".tool-versions"));
    assert_eq!(rd(&f), "python 3.12.1\n");
    // a one-line file in an ancestor is rewritten in place
    std::fs::write(proj.join(".nvmrc"), "v18\n").unwrap();
    let f = m
        .set_project_version(&sub, "node", "20.11.1", Some(&home))
        .unwrap();
    assert_eq!(f, proj.join(".nvmrc"));
    assert_eq!(rd(&f), "20.11.1\n");
    // package.json engines: .tool-versions next to it, package.json untouched
    std::fs::remove_file(proj.join(".nvmrc")).unwrap();
    let pkg = r#"{"name":"x","engines":{"node":">=18"}}"#;
    std::fs::write(proj.join("package.json"), pkg).unwrap();
    let f = m
        .set_project_version(&proj, "node", "22.1.0", Some(&home))
        .unwrap();
    assert_eq!(f, proj.join(".tool-versions"));
    assert_eq!(rd(&proj.join("package.json")), pkg);
    assert_eq!(
        crate::resolver::spec_in_dir("node", &proj).unwrap().specs,
        vec!["22.1.0"]
    );
    // existing .tool-versions keeps other tools
    std::fs::write(proj.join(".tool-versions"), "java 21\nnode 20\n").unwrap();
    m.set_project_version(&proj, "node", "22.1.0", Some(&home))
        .unwrap();
    let tv = rd(&proj.join(".tool-versions"));
    assert!(tv.contains("java 21") && tv.contains("node 22.1.0") && !tv.contains("node 20"));
    // the home directory's .tool-versions is never written
    std::fs::write(home.join(".tool-versions"), "go 1.22\n").unwrap();
    let f = m
        .set_project_version(&proj, "go", "1.23.0", Some(&home))
        .unwrap();
    assert_eq!(f, proj.join(".tool-versions"));
    assert_eq!(rd(&home.join(".tool-versions")), "go 1.22\n");
    // go.mod is not rewritten
    std::fs::write(sub.join("go.mod"), "module x\ngo 1.21\n").unwrap();
    let f = m
        .set_project_version(&sub, "go", "1.23.0", Some(&home))
        .unwrap();
    assert_eq!(f, sub.join(".tool-versions"));
    assert_eq!(rd(&sub.join("go.mod")), "module x\ngo 1.21\n");
    assert!(m
        .set_project_version(&sub, "go", "1.2 3", Some(&home))
        .is_err());
    assert!(m
        .set_project_version(&sub, "go", "../x", Some(&home))
        .is_err());
}
