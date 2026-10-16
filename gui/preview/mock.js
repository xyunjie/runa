// Browser preview of the GUI with mocked IPC (no Tauri needed).
// Used by scripts/ui-screens.sh to render screenshots; never shipped in the app bundle.
(() => {
  const H = "/home/dev/.runa";
  const params = new URLSearchParams(location.search);
  const listeners = {};
  // Screenshots: disable CSS animations so headless captures show the final state.
  if (params.get("static") === "1") {
    const st = document.createElement("style");
    st.textContent = "*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition:none!important}.bar.indeterminate>i{animation:none!important;width:35%!important}";
    document.head.appendChild(st);
  }
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const inst = (tool, v, extra = {}) => ({ version: v, source: "download", path: `${H}/installs/${tool}/${v}`, origin: null, ...extra });
  const spec = (tool, specs, kind, detail) => ({ tool, specs, source: { kind, detail } });
  const shellsConfigured = params.get("activated") !== "0";
  const ov = {
    home: H, cli_path: "/usr/local/bin/runa", cli_found: true, project_dir: "/home/dev/code/web-app", platform: "macos/arm64",
    settings: { mirror: params.get("mirror") || "off", mirror_rules: [], proxy: params.get("proxy") || "system" },
    warnings: [], is_windows: false, default_shell: "zsh", hook_active: true, app_version: "0.1.0",
    shells: [
      { shell: "bash", profile: "/home/dev/.bash_profile", configured: false },
      { shell: "zsh", profile: "/home/dev/.zshrc", configured: shellsConfigured },
      { shell: "fish", profile: "/home/dev/.config/fish/conf.d/runa.fish", configured: false },
      { shell: "powershell", profile: "/home/dev/.config/powershell/Microsoft.PowerShell_profile.ps1", configured: false },
    ],
    tools: [
      { tool: "node", description: "Node.js JavaScript 运行时（官方预编译包）", builtin: true, global: "22.23.3",
        installed: [inst("node", "22.23.3"), inst("node", "20.20.2"), inst("node", "18.20.8"), inst("node", "20.19.2", { source: "adopted", origin: "/usr/local/bin" })],
        current: { tool: "node", spec: spec("node", ["20"], "file", "/home/dev/code/web-app/.nvmrc"), version: "20.20.2", system: false } },
      { tool: "java", description: "Java JDK（Eclipse Temurin / Adoptium）", builtin: true, global: "21.0.12.1",
        installed: [inst("java", "21.0.12.1"), inst("java", "17.0.16")],
        current: { tool: "java", spec: spec("java", ["21.0.12.1"], "global", `${H}/.tool-versions`), version: "21.0.12.1", system: false } },
      { tool: "python", description: "CPython（python-build-standalone 预编译包）", builtin: true, global: "3.12.15",
        installed: [inst("python", "3.13.16"), inst("python", "3.12.15")],
        current: { tool: "python", spec: spec("python", ["3.13"], "file", "/home/dev/code/web-app/.tool-versions"), version: "3.13.16", system: false } },
      { tool: "go", description: "Go 工具链（go.dev 官方发行版）", builtin: true, global: null, installed: [],
        current: { tool: "go", spec: spec("go", ["1.27"], "file", "/home/dev/code/web-app/go.mod"), version: null, system: false } },
    ],
  };
  if (params.get("empty") === "1") ov.tools.forEach((t) => { t.installed = []; t.current = null; t.global = null; });
  const nodeVersions = [];
  for (const [maj, minors, lts] of [[26, 12, false], [25, 9, false], [24, 11, true], [23, 11, false], [22, 23, true], [21, 7, false], [20, 20, true], [18, 20, true]])
    for (let m = minors; m >= Math.max(0, minors - 5); m--) nodeVersions.push({ version: `${maj}.${m}.${m % 4}`, lts, prerelease: false });
  const remote = {
    node: nodeVersions,
    java: [27, 26, 25, 24, 23, 22, 21, 20, 17, 11, 8].map((v) => ({ version: String(v), lts: [25, 21, 17, 11, 8].includes(v), prerelease: false })),
    python: ["3.15.0", "3.14.8", "3.13.16", "3.12.15", "3.11.17", "3.10.19"].map((v) => ({ version: v, lts: false, prerelease: false })),
    go: ["1.27.2", "1.27.1", "1.27.0", "1.27rc2", "1.26.9", "1.26.8", "1.25.14"].map((v) => ({ version: v, lts: false, prerelease: v.includes("rc") })),
  };
  const usage = [];
  const sizes = { node: 190e6, java: 320e6, python: 120e6, go: 260e6 };
  ov.tools.forEach((t) => t.installed.forEach((i) => usage.push({ tool: t.tool, version: i.version, bytes: i.source === "adopted" ? 0 : sizes[t.tool] * (0.9 + Math.random() * 0.2) })));
  const emit = (ev, payload) => (listeners[ev] || []).forEach((f) => f({ payload }));

  const handlers = {
    overview: async () => { await sleep(params.get("slow") ? 1e9 : 60); return structuredClone(ov); },
    disk_usage: async () => { await sleep(120); return usage; },
    remote_versions: async ({ tool }) => { await sleep(params.get("remote") === "slow" ? 1e9 : 150); if (params.get("remote") === "error") throw "请求 https://nodejs.org/dist/index.json 失败: 连接超时（30s）"; return remote[tool] || []; },
    install: async ({ tool, spec: s }) => {
      emit("install-progress", { stage: "resolving", tool, spec: s });
      await sleep(200);
      const total = 48_600_000;
      for (let d = 0; d <= total * 0.62; d += 2_100_000) { emit("install-progress", { stage: "downloading", tool, version: { node: "22.23.3", go: "1.27.2", java: "21.0.12.1", python: "3.13.16" }[tool], done: d, total }); await sleep(90); }
      return new Promise(() => {}); // keep the job running for the screenshot
    },
    detect_runtimes: async () => { await sleep(200); return [
      { tool: "node", version: "20.19.2", exe: "/usr/local/bin/node", found_in: "/usr/local/bin", home: "/usr/local", via: "PATH", adopted: true },
      { tool: "python", version: "3.13.5", exe: "/usr/bin/python3", found_in: "/usr/bin", home: "/usr", via: "PATH", adopted: false },
      { tool: "java", version: "17.0.12", exe: "/Library/Java/JavaVirtualMachines/temurin-17.jdk/Contents/Home/bin/java", found_in: "", home: "", via: "JAVA_HOME", adopted: false },
      { tool: "go", version: "1.24.4", exe: "/opt/homebrew/bin/go", found_in: "/opt/homebrew/bin", home: "/opt/homebrew/Cellar/go/1.24.4/libexec", via: "PATH", adopted: false },
    ]; },
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { if (!handlers[cmd]) return null; return handlers[cmd](args || {}); } },
    event: { listen: async (ev, f) => { (listeners[ev] ||= []).push(f); return () => {}; } },
  };
  // scripted interactions for screenshots
  window.addEventListener("load", () => setTimeout(() => {
    const demo = params.get("demo");
    if (demo === "install") document.querySelector('[data-act="install"]')?.click();
    if (demo === "tab-installed") document.querySelector('[data-tab="installed"]')?.click();
    if (demo === "detect") document.querySelector('[data-act="detect"]')?.click();
    if (demo === "confirm") document.querySelector('[data-act="uninstall"]')?.click();
    if (demo === "toast") document.querySelector('[data-act="global"]')?.click();
    if (demo === "search") { const q = document.querySelector("[data-query]"); if (q) { q.value = params.get("q") || "1.27"; q.dispatchEvent(new Event("input", { bubbles: true })); } }
  }, 500));
})();
