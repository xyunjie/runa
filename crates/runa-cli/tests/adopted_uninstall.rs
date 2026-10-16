//! CLI behaviour for adopted versions, against a throw-away RUNA_HOME.

use std::process::Command;

fn runa(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_runa"))
        .args(args)
        .env("RUNA_HOME", home)
        .current_dir(home)
        .output()
        .unwrap()
}

#[test]
fn adopted_uninstall_requires_choice_and_dry_run_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("runa");
    let rt = t.path().join("rt").join("node-v20");
    std::fs::create_dir_all(rt.join("bin")).unwrap();
    std::fs::create_dir_all(home.join("installs/node")).unwrap();
    let receipt = serde_json::json!({
        "tool": "node", "version": "20.1.0", "source": "adopted",
        "origin": rt, "home": rt, "bin_dirs": [rt.join("bin")], "provider": "manual"
    });
    let rp = home.join("installs/node/20.1.0.receipt.json");
    std::fs::write(&rp, receipt.to_string()).unwrap();

    let o = runa(&home, &["uninstall", "node", "20.1.0"]);
    assert_eq!(o.status.code(), Some(2));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("--forget") && out.contains("--purge"), "{out}");

    let o = runa(&home, &["uninstall", "node", "20.1.0", "--purge", "--dry-run"]);
    assert!(o.status.success());
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("rm -rf"), "{out}");
    assert!(rt.is_dir() && rp.is_file(), "dry run must not delete");

    let o = runa(&home, &["disable", "node", "20.1.0"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = runa(&home, &["list"]);
    assert!(String::from_utf8_lossy(&o.stdout).contains("已禁用"));
    runa(&home, &["enable", "node@20.1.0"]);

    let o = runa(&home, &["uninstall", "node", "20.1.0", "--forget"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(rt.is_dir(), "forget keeps files");
    assert!(!rp.exists());
}
