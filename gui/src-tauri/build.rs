use std::path::Path;

fn main() {
    // The webview assets come from gui/web (React + Vite), built with `npm run build`.
    // `tauri::generate_context!` needs the dist directory to exist, so a plain `cargo build`
    // without a frontend build still compiles and shows how to build it.
    let dist = Path::new("../web/dist");
    println!("cargo:rerun-if-changed=../web/dist");
    if !dist.join("index.html").exists() {
        println!(
            "cargo:warning=gui/web/dist is missing: run `npm ci && npm run build` in gui/web to embed the UI"
        );
        std::fs::create_dir_all(dist).expect("create gui/web/dist");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=utf-8><title>Runa</title>\
             <body style=\"font:14px system-ui;padding:40px\">\
             <h3>界面尚未构建</h3><p>请在 <code>gui/web</code> 中运行 \
             <code>npm ci &amp;&amp; npm run build</code>，然后重新编译 runa-gui。</p>",
        )
        .expect("write placeholder index.html");
    }
    tauri_build::build()
}
