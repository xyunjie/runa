// Browser preview with mocked IPC (used when not running inside Tauri, e.g. `npm run dev`
// opened in a normal browser, or screenshots). Never used by the desktop app.
/* eslint-disable @typescript-eslint/no-explicit-any */
const H = "/home/dev/.runa";
const params = new URLSearchParams(typeof location === "undefined" ? "" : location.search);
const listeners: Record<string, ((p: any) => void)[]> = {};
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const emit = (ev: string, payload: unknown) => (listeners[ev] || []).forEach((f) => f(payload));

const inst = (tool: string, v: string, extra: Record<string, unknown> = {}) => ({
  version: v, source: "download", path: `${H}/installs/${tool}/${v}`, origin: null, provider: null, formula: null, disabled: false, ...extra,
});
const spec = (tool: string, specs: string[], kind: string, detail: string) => ({ tool, specs, source: { kind, detail } });

let mockProjects: string[] = ["/home/dev/code/web-app"];
let mcpPolicy: any = { mode: "read_only", confirm_destructive: true, confirm_writes: false };
const ov: any = {
  home: H, cli_path: "/usr/local/bin/runa", cli_found: true, project_dir: "/home/dev/code/web-app", platform: "macos/arm64",
  settings: { mirror: params.get("mirror") || "off", mirror_rules: [], proxy: params.get("proxy") || "system", install_via: {} },
  warnings: [], is_windows: false, default_shell: "zsh", hook_active: true, app_version: "0.1.0",
  shells: [
    { shell: "bash", profile: "/home/dev/.bash_profile", configured: false },
    { shell: "zsh", profile: "/home/dev/.zshrc", configured: params.get("activated") !== "0" },
    { shell: "fish", profile: "/home/dev/.config/fish/conf.d/runa.fish", configured: false },
  ],
  tools: [
    { tool: "node", description: "Node.js JavaScript 运行时（官方预编译包）", builtin: true, global: "22.23.3",
      installed: [inst("node", "22.23.3"), inst("node", "20.20.2"), inst("node", "20.19.2", { source: "adopted", path: "/home/dev/.nvm/versions/node/v20.19.2", provider: "nvm" }), inst("node", "18.19.1", { source: "adopted", path: "/usr", provider: "system" })],
      current: { tool: "node", spec: spec("node", ["20"], "file", "/home/dev/code/web-app/.nvmrc"), version: "20.20.2", system: false } },
    { tool: "java", description: "Java JDK（Eclipse Temurin / Adoptium）", builtin: true, global: "21.0.12.1",
      installed: [inst("java", "21.0.12.1"), inst("java", "17.0.16")],
      current: { tool: "java", spec: spec("java", ["21.0.12.1"], "global", `${H}/.tool-versions`), version: "21.0.12.1", system: false } },
    { tool: "python", description: "CPython（python-build-standalone 预编译包）", builtin: true, global: "3.12.15",
      installed: [inst("python", "3.13.16"), inst("python", "3.12.15")],
      current: { tool: "python", spec: spec("python", ["3.13"], "file", "/home/dev/code/web-app/.tool-versions"), version: "3.13.16", system: false } },
    { tool: "go", description: "Go 工具链（go.dev 官方发行版）", builtin: true, global: null, installed: [],
      current: { tool: "go", spec: spec("go", ["1.27"], "file", "/home/dev/code/web-app/go.mod"), version: null, system: false } },
    { tool: "rust", description: "Rust 工具链（通过 rustup / mise / asdf 安装）", builtin: true, global: "1.90.0",
      installed: [inst("rust", "1.90.0", { source: "adopted", path: "/home/dev/.rustup/toolchains/stable-aarch64-apple-darwin", provider: "rustup" })],
      current: { tool: "rust", spec: spec("rust", ["1.90.0"], "global", `${H}/.tool-versions`), version: "1.90.0", system: false } },
    { tool: "ruby", description: "Ruby（官方只提供源码，推荐通过 rbenv / mise 安装）", builtin: true, global: null, installed: [], current: null },
    { tool: "bun", description: "Bun JavaScript 运行时（GitHub 官方发行版）", builtin: true, global: "1.3.2", installed: [inst("bun", "1.3.2")],
      current: { tool: "bun", spec: spec("bun", ["1.3.2"], "global", `${H}/.tool-versions`), version: "1.3.2", system: false } },
    { tool: "deno", description: "Deno JavaScript / TypeScript 运行时（GitHub 官方发行版）", builtin: true, global: null, installed: [], current: null },
  ],
};
if (params.get("empty") === "1") ov.tools.forEach((t: any) => { t.installed = []; t.current = null; t.global = null; });

const nodeVersions: any[] = [];
for (const [maj, minors, lts] of [[26, 12, false], [25, 9, false], [24, 11, true], [22, 23, true], [20, 20, true], [18, 20, true]] as const)
  for (let m = minors; m >= Math.max(0, minors - 5); m--) nodeVersions.push({ version: `${maj}.${m}.${m % 4}`, lts, prerelease: false });
const remote: Record<string, any[]> = {
  node: nodeVersions,
  java: [27, 26, 25, 21, 17, 11, 8].map((v) => ({ version: String(v), lts: [25, 21, 17, 11, 8].includes(v), prerelease: false })),
  python: ["3.15.0", "3.14.8", "3.13.16", "3.12.15", "3.11.17"].map((v) => ({ version: v, lts: false, prerelease: false })),
  go: ["1.27.2", "1.27.1", "1.27rc2", "1.26.9"].map((v) => ({ version: v, lts: false, prerelease: v.includes("rc") })),
  rust: ["1.92.0", "1.91.1", "1.91.0", "1.90.0"].map((v) => ({ version: v, lts: false, prerelease: false })),
  ruby: ["3.5.0-preview1", "3.4.7", "3.3.9", "3.2.9"].map((v) => ({ version: v, lts: false, prerelease: v.includes("preview") })),
  bun: ["1.3.3", "1.3.2", "1.2.23"].map((v) => ({ version: v, lts: false, prerelease: false })),
  deno: ["2.5.6", "2.5.5", "2.1.0"].map((v) => ({ version: v, lts: false, prerelease: false })),
};
const sizes: Record<string, number> = { node: 190e6, java: 320e6, python: 120e6, go: 260e6, rust: 680e6, bun: 95e6 };
const usage = ov.tools.flatMap((t: any) => t.installed.map((i: any) => ({ tool: t.tool, version: i.version, bytes: i.source === "adopted" ? 0 : sizes[t.tool] })));

const detected = () => [
  { tool: "node", version: "20.19.2", exe: "/home/dev/.nvm/versions/node/v20.19.2/bin/node", found_in: "", home: "/home/dev/.nvm/versions/node/v20.19.2", via: "nvm", provider: "nvm", formula: null, adopted: true, adopted_as: null, disabled: false, system_default: true, runa_global: false, view_only: false },
  { tool: "python", version: "3.13.5", exe: "/usr/bin/python3", found_in: "/usr/bin", home: "/usr", via: "PATH", provider: "system", formula: null, adopted: false, adopted_as: null, disabled: false, system_default: false, runa_global: false, view_only: true },
  { tool: "go", version: "1.24.4", exe: "/opt/homebrew/bin/go", found_in: "/opt/homebrew/bin", home: "/opt/homebrew/Cellar/go/1.24.4/libexec", via: "PATH", provider: "homebrew", formula: "go", adopted: false, adopted_as: null, disabled: false, system_default: false, runa_global: false, view_only: false },
];
const choice = (tool: string, preference = ov.settings.install_via[tool] || "auto") => {
  const options = tool === "node"
    ? [{ via: "nvm", available: true, detail: "/home/dev/.nvm" }, { via: "fnm", available: false, detail: "未安装" }, { via: "runa", available: true, detail: "" }]
    : tool === "ruby"
      ? [{ via: "runa", available: false, detail: "没有可移植的官方预编译包" }, { via: "rbenv", available: false, detail: "未检测到" }, { via: "rvm", available: false, detail: "未检测到" }, { via: "mise", available: false, detail: "未检测到" }, { via: "asdf", available: false, detail: "未检测到" }]
      : tool === "rust"
        ? [{ via: "runa", available: false, detail: "runa 不直接下载 Rust" }, { via: "rustup", available: params.get("norustup") !== "1", detail: "/home/dev/.cargo/bin/rustup" }, { via: "mise", available: false, detail: "未检测到" }]
        : [{ via: "runa", available: true, detail: "" }];
  const auto = options.find((o) => o.via !== "runa" && o.available)?.via || (options.find((o) => o.via === "runa")?.available ? "runa" : "auto（不可用）");
  return { tool, preference, chosen: preference === "auto" ? auto : preference, options };
};
const managers = () => [
  { name: "nvm", tools: ["node"], supported: true, installed: true, version: "0.40.3", entry: "/home/dev/.nvm/nvm.sh", data_dir: "/home/dev/.nvm", origin: "script", runtimes: [{ tool: "node", version: "20.19.2", global: false }], install_method: null },
  { name: "fnm", tools: ["node"], supported: true, installed: false, version: null, entry: null, data_dir: null, origin: "-", runtimes: [], install_method: { manager: "fnm", method: "brew", program: "brew", args: [], display: "brew install fnm", rc_note: "", note: "" } },
  { name: "pyenv", tools: ["python"], supported: true, installed: true, version: "2.6.27", entry: "/opt/homebrew/bin/pyenv", data_dir: "/home/dev/.pyenv", origin: "brew", runtimes: [], install_method: null },
  { name: "rustup", tools: ["rust"], supported: true, installed: true, version: "1.28.2", entry: "/home/dev/.cargo/bin/rustup", data_dir: "/home/dev/.rustup", origin: "script", runtimes: [{ tool: "rust", version: "1.90.0", global: true }], install_method: null },
  { name: "rbenv", tools: ["ruby"], supported: true, installed: false, version: null, entry: null, data_dir: null, origin: "-", runtimes: [], install_method: { manager: "rbenv", method: "brew", program: "brew", args: ["install", "rbenv", "ruby-build"], display: "brew install rbenv ruby-build", rc_note: "", note: "" } },
  { name: "rvm", tools: ["ruby"], supported: true, installed: false, version: null, entry: null, data_dir: null, origin: "-", runtimes: [], install_method: null },
  { name: "nvm-windows", tools: ["node"], supported: false, installed: false, version: null, entry: null, data_dir: null, origin: "-", runtimes: [], install_method: null },
];
async function streamLines(name: string, n = 6) {
  for (let i = 1; i <= n; i++) { emit("manager-progress", { name, line: `step ${i}/${n}` }); await sleep(250); }
}

const handlers: Record<string, (a: any) => Promise<unknown>> = {
  overview: async () => { await sleep(params.get("slow") ? 1e9 : 60); return structuredClone(ov); },
  disk_usage: async () => { await sleep(120); return usage; },
  remote_versions: async ({ tool }) => {
    await sleep(params.get("remote") === "slow" ? 1e9 : 150);
    if (params.get("remote") === "error") throw "请求 https://nodejs.org/dist/index.json 失败: 连接超时（30s）";
    return remote[tool] || [];
  },
  install: async ({ tool, spec: s }) => {
    emit("install-progress", { stage: "resolving", tool, spec: s });
    await sleep(200);
    const total = 48_600_000;
    for (let d = 0; d <= total; d += 4_100_000) { emit("install-progress", { stage: "downloading", tool, version: s, done: d, total }); await sleep(90); }
    emit("install-progress", { stage: "extracting", tool, version: s });
    await sleep(300);
    return { version: s, via: "runa" };
  },
  installer_choice: async ({ tool }) => choice(tool),
  set_install_via: async ({ tool, via }) => { ov.settings.install_via[tool] = via; return choice(tool, via); },
  manager_install_plan: async ({ name }) => ({ manager: name, method: "brew", program: "brew", args: ["install", name], display: `brew install ${name}`, rc_note: "", note: "" }),
  install_manager: async ({ name, tool }) => { await streamLines(name); return choice(tool, name); },
  install_manager_only: async ({ name }) => { await streamLines(name); return null; },
  manager_info: async () => { await sleep(100); return managers(); },
  manager_plan: async ({ name, action, keepData }) => {
    const m = managers().find((x) => x.name === name)!;
    const kd = !!keepData && action === "uninstall";
    return {
      manager: name, action, kind: m.origin === "unknown" ? "refuse" : "run", origin: m.origin, command: null, env: [],
      command_line: action === "upgrade" ? (m.origin === "brew" ? `brew upgrade ${name}` : "curl … | bash") : m.origin === "brew" ? `brew uninstall ${name}` : null,
      remove_dirs: action === "uninstall" ? (kd ? [`${m.data_dir}/nvm.sh`] : [m.data_dir]) : [],
      rc_edits: action === "uninstall" ? [{ file: "/home/dev/.zshrc", lines: [[12, 'export NVM_DIR="$HOME/.nvm"'], [13, '[ -s "$NVM_DIR/nvm.sh" ] && \\. "$NVM_DIR/nvm.sh"']] }] : [],
      runtimes: action === "uninstall" ? m.runtimes : [],
      data_runtimes: action === "uninstall" && name === "nvm" ? ["/home/dev/.nvm/versions/node/v20.19.2"] : [],
      keep_data: kd,
      summary: action === "upgrade" ? `升级 ${name}` : `卸载 ${name}${kd ? "（保留已安装的版本）" : ""}`,
    };
  },
  manager_execute: async ({ name, keepData }) => { await streamLines(name); return { unregistered: [], notices: keepData ? ["已保留 1 个运行时，来源改为“手动安装”"] : [] }; },
  detect_cached: async () => ({ items: detected(), from_cache: true, stale: false, created_at: Date.now() / 1000 - 600 }),
  detect_refresh: async () => { await sleep(400); return { items: detected(), from_cache: false, stale: false, created_at: Date.now() / 1000 }; },
  detect_check: async () => false,
  migrate_adoptions: async () => [],
  set_disabled: async () => [],
  set_global: async ({ tool, version }) => { ov.tools.find((t: any) => t.tool === tool).global = version; return version; },
  set_local: async () => "/home/dev/code/web-app/.tool-versions",
  unset_local: async () => null,
  adopt: async ({ item }) => ({ version: item.version, set_global: false }),
  adopt_all: async () => [],
  set_mirror: async ({ mode }) => { ov.settings.mirror = mode; return structuredClone(ov.settings); },
  set_proxy: async ({ value }) => { ov.settings.proxy = value; return structuredClone(ov.settings); },
  setup_shell: async ({ shell }) => `/home/dev/.${shell}rc`,
  remove_shell: async () => null,
  add_user_path: async () => true,
  reshim: async () => 12,
  uninstall: async () => null,
  mcp_status: async () => ({
    cli_path: "/usr/local/bin/runa", cli_found: true, bridge_running: true, bridge_error: null, policy: mcpPolicy,
    clients: [
      ["cursor", "Cursor", "deeplink", "/home/dev/.cursor/mcp.json", true],
      ["vscode", "VS Code", "deeplink", "/home/dev/.config/Code/User/mcp.json", false],
      ["claude-desktop", "Claude Desktop", "file", "/home/dev/.config/Claude/claude_desktop_config.json", false],
      ["claude-code", "Claude Code", "cli", "/home/dev/.claude.json", false],
      ["codex", "Codex", "file", "/home/dev/.codex/config.toml", false],
      ["windsurf", "Windsurf", "file", "/home/dev/.codeium/windsurf/mcp_config.json", false],
      ["opencode", "OpenCode", "file", "/home/dev/.config/opencode/opencode.json", false],
    ].map(([id, name, method, config_path, added]) => ({
      id, name, method, config_path, added, link: method === "deeplink" ? "#" : null, cli: id === "claude-code" ? "/home/dev/.local/bin/claude" : null,
      command_line: id === "claude-code" ? "claude mcp add --scope user runa -- /usr/local/bin/runa mcp" : null,
      snippet: id === "codex" ? '[mcp_servers.runa]\ncommand = "/usr/local/bin/runa"\nargs = ["mcp"]\n' : '{\n  "mcpServers": {\n    "runa": { "command": "/usr/local/bin/runa", "args": ["mcp"] }\n  }\n}\n',
    })),
  }),
  projects_list: async () => mockProjects,
  project_add: async ({ paths }) => { mockProjects = [...new Set([...mockProjects, ...paths])]; return mockProjects; },
  project_remove: async ({ path }) => { mockProjects = mockProjects.filter((p) => p !== path); return mockProjects; },
  project_detail: async ({ path }) => ({ path, name: path.split("/").pop(), exists: true, items: [
    { tool: "node", required: "20", source: `${path}/.nvmrc`, file: `${path}/.nvmrc`, status: "ok", resolved: "20.20.2", installed: ["22.23.3", "20.20.2"] },
    { tool: "python", required: "3.11", source: `${path}/.python-version`, file: `${path}/.python-version`, status: "mismatch", resolved: null, installed: ["3.13.16", "3.12.15"] },
    { tool: "go", required: "1.27", source: `${path}/go.mod`, file: `${path}/go.mod`, status: "missing", resolved: null, installed: [] },
  ] }),
  project_set_version: async ({ path }) => `${path}/.tool-versions`,
  project_scan: async ({ root }) => [`${root}/web-app`, `${root}/api`],
  mcp_set_policy: async ({ policy }) => { mcpPolicy = policy; return policy; },
  mcp_add: async () => "已添加",
  mcp_remove: async () => "已移除",
  mcp_pending: async () => params.get("approval") ? [{ id: "demo", op: { kind: "uninstall", tool: "node", version: "20.20.2", via: null, mode: "auto" }, summary: "卸载 node 20.20.2：删除 /home/dev/.runa/installs/node/20.20.2", details: ["删除 /home/dev/.runa/installs/node/20.20.2"], client: "cursor" }] : [],
  mcp_respond: async () => null,
  uninstall_plan: async ({ tool, version, mode }) => ({
    tool, version, kind: mode === "forget" ? "forget" : mode === "purge" ? "provider_command" : "delete_install",
    provider: mode === "auto" ? null : "nvm", command: null, env: [],
    command_line: mode === "purge" ? `nvm uninstall ${version}` : null, paths: [`${H}/installs/${tool}/${version}`],
    home: mode === "forget" ? `/home/dev/.nvm/versions/node/v${version}` : null, needs_root: false,
    summary: mode === "forget" ? "只移除 runa 的管理记录" : mode === "purge" ? `通过 nvm 彻底卸载：nvm uninstall ${version}` : `删除 ${H}/installs/${tool}/${version}`,
  }),
};

export async function mockInvoke<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  const h = handlers[cmd];
  return (h ? await h(args) : null) as T;
}
export async function mockListen<T>(event: string, f: (p: T) => void): Promise<() => void> {
  (listeners[event] ||= []).push(f as (p: unknown) => void);
  return () => { listeners[event] = (listeners[event] || []).filter((x) => x !== f); };
}
