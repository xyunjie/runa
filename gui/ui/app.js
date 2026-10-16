// Runa desktop UI. Vanilla JS; talks to the Rust core over Tauri IPC.
import { icon, mark } from "./icons.js";

const T = window.__TAURI__;
const invoke = (cmd, args = {}) => T.core.invoke(cmd, args);

// ---------------------------------------------------------------- state
const S = {
  installer: {}, // tool -> InstallerChoice from backend
  mgrJob: null, // {name, line} while a version manager is being installed
  ov: null, // overview from backend
  loading: true,
  loadError: null,
  usage: null, // [{tool, version, bytes}]
  remote: {}, // tool -> {status: loading|ok|error, items, error}
  tab: {}, // tool -> installed | available
  query: {}, // tool -> search text
  filter: {}, // tool -> all | lts | stable
  pageSize: {}, // tool -> n shown
  jobs: {}, // tool -> progress info
  detected: { status: "idle", items: [], error: null, createdAt: 0, refreshing: false },
  migration: [], // pending legacy (symlink) adoptions, from a dry-run
  proxyDraft: null,
  proxyError: null,
};

const $ = (s, el = document) => el.querySelector(s);
const esc = (v) => String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const fmtBytes = (n) => {
  if (n == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(n < 10 && i ? 1 : 0)} ${u[i]}`;
};
const shortPath = (p) => {
  const h = S.ov?.home || "";
  if (!/[\\/]\.runa$/.test(h)) return p; // custom RUNA_HOME: show full paths
  const home = h.replace(/[\\/]\.runa$/, "");
  return p?.startsWith(home) ? "~" + p.slice(home.length) : p;
};
const fileName = (p) => String(p || "").split(/[\\/]/).pop();
function hydrateIcons(root = document) {
  root.querySelectorAll("[data-icon]:not([data-done])").forEach((el) => {
    el.innerHTML = icon(el.dataset.icon, Number(el.dataset.size || 16));
    el.dataset.done = "1";
  });
}

// ---------------------------------------------------------------- theme
const THEMES = [["system", "monitor", "跟随系统"], ["light", "sun", "浅色"], ["dark", "moon", "深色"]];
const mql = window.matchMedia("(prefers-color-scheme: dark)");
function themePref() {
  const q = new URLSearchParams(location.search).get("theme");
  return q || localStorage.getItem("runa.theme") || "system";
}
function applyTheme() {
  const p = themePref();
  const dark = p === "dark" || (p === "system" && mql.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  $("#theme-seg").innerHTML = THEMES.map(([k, ic, label]) =>
    `<button role="radio" aria-checked="${p === k}" data-theme-set="${k}" title="${label}" aria-label="${label}">${icon(ic, 14)}</button>`).join("");
}
mql.addEventListener("change", applyTheme);

// ---------------------------------------------------------------- toasts & dialog
function toast(title, detail = "", kind = "ok") {
  const el = document.createElement("div");
  el.className = `toast ${kind === "error" ? "error" : ""}`;
  el.setAttribute("role", kind === "error" ? "alert" : "status");
  el.innerHTML = `<span class="ti">${icon(kind === "error" ? "alert" : "checkCircle", 16)}</span>
    <div class="msg"><b>${esc(title)}</b>${detail ? esc(detail) : ""}</div>
    <button class="btn btn-ghost btn-sm btn-icon" aria-label="关闭">${icon("x", 14)}</button>`;
  const close = () => { el.classList.add("out"); setTimeout(() => el.remove(), 180); };
  el.querySelector("button").onclick = close;
  $("#toasts").appendChild(el);
  setTimeout(close, kind === "error" ? 8000 : 3200);
}
const fail = (title) => (e) => { toast(title, String(e), "error"); throw e; };

function confirmDialog({ title, body, ok = "确定", danger = false }) {
  return new Promise((resolve) => {
    const root = $("#modal-root");
    const prev = document.activeElement;
    root.innerHTML = `<div class="backdrop"><div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="dlg-t">
      <h3 id="dlg-t">${esc(title)}</h3><p>${body}</p>
      <div class="row"><button class="btn" data-r="0">取消</button>
      <button class="btn ${danger ? "btn-danger-solid" : "btn-primary"}" data-r="1">${esc(ok)}</button></div></div></div>`;
    const done = (v) => { root.innerHTML = ""; document.removeEventListener("keydown", onKey); prev?.focus?.(); resolve(v); };
    const onKey = (e) => {
      if (e.key === "Escape") done(false);
      if (e.key === "Tab") { // trap focus between the two buttons
        const b = root.querySelectorAll("button");
        if (e.shiftKey && document.activeElement === b[0]) { e.preventDefault(); b[1].focus(); }
        else if (!e.shiftKey && document.activeElement === b[1]) { e.preventDefault(); b[0].focus(); }
      }
    };
    document.addEventListener("keydown", onKey);
    root.querySelector(".backdrop").addEventListener("mousedown", (e) => { if (e.target.classList.contains("backdrop")) done(false); });
    root.querySelectorAll("[data-r]").forEach((b) => (b.onclick = () => done(b.dataset.r === "1")));
    root.querySelector('[data-r="0"]').focus();
  });
}

const PROVIDERS = { homebrew: "Homebrew", nvm: "nvm", pyenv: "pyenv", sdkman: "SDKMAN!", asdf: "asdf", fnm: "fnm", manual: "手动安装", system: "系统自带", "nvm-windows": "nvm-windows", "pyenv-win": "pyenv-win", mise: "mise", scoop: "Scoop", "scoop-global": "Scoop（全局）", winget: "winget", chocolatey: "Chocolatey", installer: "安装程序" };
const providerBadge = (p, formula) => p ? `<span class="badge" title="${esc(formula ? `${p}: ${formula}` : p)}">${esc(PROVIDERS[p] || p)}${formula ? ` · <span class="mono">${esc(formula)}</span>` : ""}</span>` : "";
const ago = (secs) => {
  if (!secs) return "";
  const d = Math.max(0, Date.now() / 1000 - secs);
  return d < 60 ? "刚刚" : d < 3600 ? `${Math.floor(d / 60)} 分钟前` : d < 86400 ? `${Math.floor(d / 3600)} 小时前` : `${Math.floor(d / 86400)} 天前`;
};

/** Generic dialog: options = [{key, title, desc}], onPick(key) -> {html, okDisabled, okLabel, danger} */
function choiceDialog({ title, intro = "", options = [], initial, describe }) {
  return new Promise((resolve) => {
    const root = $("#modal-root");
    const prev = document.activeElement;
    let cur = initial ?? options[0]?.key ?? null;
    let seq = 0;
    root.innerHTML = `<div class="backdrop"><div class="dialog wide" role="alertdialog" aria-modal="true" aria-labelledby="dlg-t">
      <h3 id="dlg-t">${esc(title)}</h3>${intro ? `<p>${intro}</p>` : ""}
      ${options.length ? `<div class="options">${options.map((o) => `<label class="option" data-opt="${esc(o.key)}"><input type="radio" name="dlg-opt" value="${esc(o.key)}"/><span class="radio"></span><div><div class="t">${esc(o.title)}</div><div class="d">${esc(o.desc)}</div></div></label>`).join("")}</div>` : ""}
      <div class="dlg-detail"></div>
      <div class="row"><button class="btn" data-r="0">取消</button><button class="btn" data-r="1">确定</button></div></div></div>`;
    const okBtn = root.querySelector('[data-r="1"]');
    const paint = async () => {
      root.querySelectorAll("[data-opt]").forEach((el) => {
        el.classList.toggle("selected", el.dataset.opt === cur);
        el.querySelector("input").checked = el.dataset.opt === cur;
      });
      const my = ++seq;
      okBtn.disabled = true;
      root.querySelector(".dlg-detail").innerHTML = `<div class="plan-box"><span class="skel" style="display:block;width:70%;height:10px"></span></div>`;
      const d = await describe(cur).catch((e) => ({ html: `<div class="plan-box danger">${esc(String(e))}</div>`, okDisabled: true }));
      if (my !== seq) return;
      root.querySelector(".dlg-detail").innerHTML = d.html || "";
      okBtn.textContent = d.okLabel || "确定";
      okBtn.className = `btn ${d.danger ? "btn-danger-solid" : "btn-primary"}`;
      okBtn.disabled = !!d.okDisabled;
    };
    const done = (v) => { root.innerHTML = ""; document.removeEventListener("keydown", onKey); prev?.focus?.(); resolve(v); };
    const onKey = (e) => { if (e.key === "Escape") done(null); };
    document.addEventListener("keydown", onKey);
    root.querySelector(".backdrop").addEventListener("mousedown", (e) => { if (e.target.classList.contains("backdrop")) done(null); });
    root.querySelectorAll("[data-opt]").forEach((el) => el.addEventListener("click", (e) => { e.preventDefault(); if (cur !== el.dataset.opt) { cur = el.dataset.opt; paint(); } }));
    root.querySelector('[data-r="0"]').onclick = () => done(null);
    okBtn.onclick = () => done(cur ?? true);
    root.querySelector('[data-r="0"]').focus();
    paint();
  });
}

function planHtml(p) {
  const rows = [];
  rows.push(`<div class="lbl">${esc(p.summary)}</div>`);
  if (p.command_line) rows.push(`<div class="lbl">${p.needs_root ? "需要管理员权限，runa 不会自动 sudo，请在终端手动执行：" : "将执行命令："}</div><span class="mono">${esc(p.command_line)}</span>`);
  if (p.paths?.length) rows.push(`<div class="lbl">${p.kind === "forget" ? "将删除的 runa 记录：" : "将删除："}</div>${p.paths.map((x) => `<span class="mono">${esc(shortPath(x))}</span>`).join("")}`);
  if (p.kind === "forget" && p.home) rows.push(`<div class="lbl">保留原有安装：</div><span class="mono">${esc(shortPath(p.home))}</span>`);
  return `<div class="plan-box ${p.kind === "forget" ? "" : "danger"}">${rows.join("")}</div>`;
}

// ---------------------------------------------------------------- data
function projectDir() { return $("#project").value.trim(); }

async function load({ quiet = false } = {}) {
  if (!quiet) { S.loading = !S.ov; render(); }
  try {
    S.ov = await invoke("overview", { project: projectDir() || null });
    S.loadError = null;
    if (!$("#project").value) $("#project").value = S.ov.project_dir;
  } catch (e) {
    S.loadError = String(e);
  }
  S.loading = false;
  render();
  invoke("disk_usage").then((u) => { S.usage = u; renderNav(); if (route().name !== "settings") render(); }).catch(() => {});
}

function usageOf(tool, version) {
  if (!S.usage) return null;
  return S.usage.filter((u) => u.tool === tool && (version == null || u.version === version)).reduce((a, u) => a + u.bytes, 0);
}

async function loadInstaller(tool) {
  try { S.installer[tool] = await invoke("installer_choice", { tool }); render(); } catch { /* optional */ }
}

function installerPicker(tool) {
  const c = S.installer[tool];
  if (!c) return "";
  const label = (v) => v === "runa" ? "runa 下载" : (PROVIDERS[v] || v);
  const opts = [["auto", `自动（${label(c.chosen)}）`],
    ...c.options.map((o) => o.available ? [o.via, label(o.via)] : [`install:${o.via}`, `${label(o.via)} — 安装此管理器…`])];
  const job = S.mgrJob;
  return `<label class="installer-pick" title="新版本的安装方式：自动时优先使用已检测到的版本管理器，安装后自动登记，卸载也会通过它">安装方式
    <select class="input" data-installer="${esc(tool)}" style="width:auto;padding:4px 8px" ${job ? "disabled" : ""}>${opts.map(([k, l]) => `<option value="${esc(k)}" ${c.preference === k ? "selected" : ""}>${esc(l)}</option>`).join("")}</select></label>
    ${job ? `<span class="muted mgr-job">${icon("refresh", 12, "spin")}正在安装 ${esc(label(job.name))}${job.line ? "：" + esc(job.line) : "…"}</span>` : ""}`;
}

async function installManager(tool, name) {
  let plan;
  try { plan = await invoke("manager_install_plan", { name }); }
  catch (e) { toast(`无法安装 ${name}`, String(e), "error"); return; }
  const body = `将执行以下命令（不会使用 sudo）：<br><code class="mono" style="display:block;white-space:pre-wrap;margin:8px 0">${esc(plan.display)}</code>
    ${plan.rc_note ? `<b>会修改：</b>${esc(plan.rc_note)}` : "不会修改 shell 配置文件。"}${plan.note ? `<br><span class="muted">${esc(plan.note)}</span>` : ""}`;
  if (!(await confirmDialog({ title: `安装 ${PROVIDERS[name] || name}`, body, ok: "安装" }))) return;
  S.mgrJob = { name, line: "" };
  render();
  try {
    S.installer[tool] = await invoke("install_manager", { name, tool });
    toast(`${PROVIDERS[name] || name} 已安装`, `已设为 ${tool} 的安装方式，现在可以通过它安装新版本`);
  } catch (e) {
    toast(`安装 ${name} 失败`, String(e), "error");
  } finally {
    S.mgrJob = null;
    render();
  }
}

function onManagerProgress(p) {
  if (!S.mgrJob || S.mgrJob.name !== p.name) return;
  S.mgrJob.line = p.line;
  const el = document.querySelector(".mgr-job");
  if (el) el.innerHTML = `${icon("refresh", 12, "spin")}正在安装 ${esc(PROVIDERS[p.name] || p.name)}：${esc(p.line)}`;
}

async function loadRemote(tool, force = false) {
  if (!force && S.remote[tool]?.status === "ok") return;
  S.remote[tool] = { status: "loading" };
  render();
  try {
    S.remote[tool] = { status: "ok", items: await invoke("remote_versions", { tool }) };
  } catch (e) {
    S.remote[tool] = { status: "error", error: String(e) };
  }
  render();
}

// ---------------------------------------------------------------- routing
function route() {
  const [, name = "overview", arg] = (location.hash || "#/overview").split("/");
  return { name, arg };
}
window.addEventListener("hashchange", () => { render(); $("#content").scrollTop = 0; });

// ---------------------------------------------------------------- helpers for tool state
function toolStatus(t) {
  if (S.jobs[t.tool]) return "busy";
  const c = t.current;
  if (c && (c.version || c.system)) return "ok";
  if (c && !c.version) return "warn";
  return "";
}
function sourceInfo(cur) {
  if (!cur) return null;
  const s = cur.spec.source;
  if (s.kind === "env") return { icon: "terminal", label: s.detail, title: `环境变量 ${s.detail}` };
  if (s.kind === "global") return { icon: "globe", label: "全局", title: s.detail };
  return { icon: "file", label: fileName(s.detail), title: s.detail };
}
function sourceChip(cur) {
  const i = sourceInfo(cur);
  if (!i) return `<span class="muted">未设置</span>`;
  return `<span class="chip" title="${esc(i.title)}">${icon(i.icon, 13)}<span class="mono">${esc(i.label)}</span></span>`;
}
function activationOk() { return S.ov.shells.some((s) => s.configured); }

// ---------------------------------------------------------------- render: chrome
function renderNav() {
  if (!S.ov) { $("#nav-tools").innerHTML = [1, 2, 3, 4].map(() => `<div class="nav-item"><span class="skel" style="width:20px;height:20px"></span><span class="skel" style="flex:1;height:10px"></span></div>`).join(""); return; }
  const r = route();
  $("#nav-tools").innerHTML = S.ov.tools.map((t) => {
    const v = t.current?.version || (t.current?.system ? "system" : "");
    return `<a class="nav-item ${r.name === "tool" && r.arg === t.tool ? "active" : ""}" href="#/tool/${esc(t.tool)}">
      ${mark(t.tool, 20)}<span class="grow">${esc(t.tool)}</span><span class="nav-ver">${esc(v)}</span><span class="dot ${toolStatus(t)}"></span></a>`;
  }).join("");
  document.querySelectorAll(".nav > .nav-item[data-route]").forEach((a) => a.classList.toggle("active", a.dataset.route === r.name));
}

function renderCrumb() {
  const r = route();
  const names = { overview: "概览", detect: "检测与接管", settings: "设置" };
  $("#crumb").innerHTML = r.name === "tool"
    ? `<span>运行时</span>${icon("chevron", 14)}<b>${esc(r.arg)}</b>`
    : `<b>${names[r.name] || "概览"}</b>`;
}

let lastRouteKey = null;
function render() {
  renderNav();
  renderCrumb();
  const c = $("#content");
  if (S.loading) { c.innerHTML = skeletonPage(); return; }
  if (S.loadError) {
    c.innerHTML = `<div class="page">${emptyState("alert", "无法读取 runa 数据", S.loadError, `<button class="btn btn-primary" data-act="reload">${icon("refresh", 14)}重试</button>`)}</div>`;
    return;
  }
  const r = route();
  const html = r.name === "tool" ? pageTool(r.arg) : r.name === "detect" ? pageDetect() : r.name === "settings" ? pageSettings() : pageOverview();
  // keep focus & caret inside the search box across re-renders
  const active = document.activeElement;
  const keep = active?.id && c.contains(active) ? { id: active.id, pos: active.selectionStart } : null;
  c.innerHTML = html;
  const key = `${r.name}/${r.arg || ""}`;
  if (key !== lastRouteKey) { c.firstElementChild?.classList.add("animate"); lastRouteKey = key; }
  hydrateIcons(c);
  if (keep) { const el = document.getElementById(keep.id); if (el) { el.focus(); try { el.setSelectionRange(keep.pos, keep.pos); } catch (_) {} } }
}

function skeletonPage() {
  const row = `<div class="item"><span class="skel" style="width:28px;height:28px;border-radius:8px"></span><div class="main-col"><div class="skel" style="width:30%;height:11px"></div><div class="skel" style="width:55%;height:9px;margin-top:8px"></div></div></div>`;
  return `<div class="page"><div class="skel" style="width:180px;height:24px"></div><div class="skel" style="width:300px;height:12px;margin:12px 0 32px"></div>
    <div class="stats">${[1, 2, 3, 4].map(() => `<div class="card stat"><div class="skel" style="width:60%;height:10px"></div><div class="skel" style="width:40%;height:20px;margin-top:12px"></div></div>`).join("")}</div>
    <div style="height:32px"></div><div class="list">${row.repeat(4)}</div></div>`;
}
function emptyState(ic, title, desc, action = "") {
  return `<div class="empty"><div class="ic">${icon(ic, 20)}</div><div class="t">${esc(title)}</div><div class="d">${esc(desc)}</div>${action}</div>`;
}

// ---------------------------------------------------------------- page: overview
function pageOverview() {
  const ov = S.ov;
  const versions = ov.tools.reduce((a, t) => a + t.installed.length, 0);
  const active = ov.tools.filter((t) => t.current?.version).length;
  const total = S.usage ? S.usage.reduce((a, u) => a + u.bytes, 0) : null;
  const act = activationOk();
  const callout = act ? "" : `<div class="callout"><span data-icon="zap"></span>
      <div class="grow"><div class="t">自动切换尚未启用</div><div class="d">启用后，进入项目目录时会根据 .tool-versions / .nvmrc 等文件自动切换版本。</div></div>
      <button class="btn btn-primary" data-act="fix-shell" ${ov.cli_found ? "" : "disabled"}>为 ${esc(ov.default_shell)} 启用</button></div>`;
  const cliWarn = ov.cli_found ? "" : `<div class="callout warn"><span data-icon="alert"></span><div class="grow"><div class="t">未找到 runa 命令行程序</div>
      <div class="d">Shell 激活与 shims 依赖 CLI。请将 runa 放入 PATH，或与桌面端放在同一目录。</div></div></div>`;
  const rows = ov.tools.map((t) => {
    const v = t.current?.version || (t.current?.system ? "system" : null);
    const missing = t.current && !t.current.version && !t.current.system;
    const size = usageOf(t.tool);
    return `<a class="item link" href="#/tool/${esc(t.tool)}">
      ${mark(t.tool, 32)}
      <div class="main-col"><div class="title">${esc(t.tool)}${missing ? `<span class="badge warn">需要 ${esc(t.current.spec.specs[0])}</span>` : ""}</div>
      <div class="meta">${esc(t.description)}</div></div>
      <div style="width:140px">${v ? `<div class="ver">${esc(v)}</div>` : missing ? `<div style="color:var(--warn)">未安装</div>` : `<div class="muted">未设置</div>`}</div>
      <div style="width:150px">${sourceChip(t.current)}</div>
      <div class="muted" style="width:90px;text-align:right;font-size:12.5px">${t.installed.length} 个版本<br/>${S.usage ? fmtBytes(size) : ""}</div>
      <span class="chev">${icon("chevron", 16)}</span></a>`;
  }).join("");
  return `<div class="page">
    <div class="page-head"><div><h1 class="page-title">概览</h1><div class="page-sub">所有运行时的当前版本与环境状态</div></div></div>
    ${cliWarn}${callout}
    <div class="stats">
      <div class="card stat"><div class="label">${icon("layers", 14)}已安装版本</div><div class="value">${versions}</div></div>
      <div class="card stat"><div class="label">${icon("checkCircle", 14)}已生效工具</div><div class="value">${active}<span class="muted" style="font-size:14px;font-weight:400"> / ${ov.tools.length}</span></div></div>
      <div class="card stat"><div class="label">${icon("disk", 14)}占用空间</div><div class="value">${total == null ? `<span class="skel" style="display:inline-block;width:70px;height:18px"></span>` : fmtBytes(total)}</div></div>
      <div class="card stat"><div class="label">${icon("terminal", 14)}自动切换</div><div class="value small">${act ? `<span class="badge ok">${icon("check", 12)}已启用</span>` : `<span class="badge warn">未启用</span>`}</div></div>
    </div>
    <div class="section-title">运行时 <a class="btn btn-ghost btn-sm" href="#/detect">${icon("scan", 14)}检测已有安装</a></div>
    <div class="list">${rows}</div>
  </div>`;
}

// ---------------------------------------------------------------- page: tool
function pageTool(name) {
  const t = S.ov.tools.find((x) => x.tool === name);
  if (!t) return `<div class="page">${emptyState("package", "未知工具", name)}</div>`;
  const tab = S.tab[name] || (t.installed.length ? "installed" : "available");
  if (tab === "available" && !S.remote[name]) setTimeout(() => loadRemote(name));
  if (tab === "available" && !S.installer[name]) setTimeout(() => loadInstaller(name));
  const cur = t.current;
  let activeHtml;
  if (!cur) activeHtml = `<div class="label">当前版本</div><div class="muted">未设置</div>`;
  else if (cur.system) activeHtml = `<div class="label">当前版本</div><div class="ver-lg">system</div><div class="src">${sourceChip(cur)}</div>`;
  else if (cur.version) activeHtml = `<div class="label">当前版本</div><div class="ver-lg">${esc(cur.version)}</div><div class="src">${sourceChip(cur)}</div>`;
  else activeHtml = `<div class="label">需要但未安装</div><div class="ver-lg" style="color:var(--warn)">${esc(cur.spec.specs[0])}</div>
      <div class="src" style="gap:6px">${sourceChip(cur)}<button class="btn btn-primary btn-sm" data-act="install" data-tool="${esc(name)}" data-v="${esc(cur.spec.specs[0])}" ${S.jobs[name] ? "disabled" : ""}>${icon("download", 13)}安装</button></div>`;

  return `<div class="page">
    <div class="tool-head">${mark(name, 48)}
      <div class="grow"><h1 class="page-title">${esc(name)}</h1><div class="page-sub">${esc(t.description)}${t.builtin ? "" : " · JSON 插件"}</div></div>
      <div class="active-box">${activeHtml}</div></div>
    ${jobCard(name)}
    <div class="tabs-row">
      <div class="seg" role="tablist">
        <button role="tab" aria-selected="${tab === "installed"}" data-tab="installed" data-tool="${esc(name)}">已安装 <span class="count">${t.installed.length}</span></button>
        <button role="tab" aria-selected="${tab === "available"}" data-tab="available" data-tool="${esc(name)}">可安装</button>
      </div>
      ${tab === "available" ? filterSeg(name) : ""}
    </div>
    ${tab === "installed" ? installedList(t) : availableList(t)}
  </div>`;
}

function jobCard(tool) {
  const j = S.jobs[tool];
  if (!j) return "";
  const pct = j.total ? Math.min(100, Math.floor((j.done / j.total) * 100)) : null;
  const indet = j.stage !== "downloading" || pct == null;
  const stageText = { resolving: "正在解析版本…", downloading: "正在下载", verifying: "正在校验 SHA256…", extracting: "正在解压…", manager: `通过 ${j.via || "版本管理器"} 安装：${j.command || ""}`, output: `通过 ${j.via || "版本管理器"} 安装：${j.line || ""}`, done: "完成" }[j.stage] || "准备中…";
  return `<div class="card progress-card" role="status" aria-live="polite">
    <div class="progress-top"><div class="t">${icon("refresh", 14, "spin")}安装 ${esc(tool)} ${esc(j.version || j.spec || "")}</div>
      <div class="n">${pct != null && j.stage === "downloading" ? pct + "%" : ""}</div></div>
    <div class="bar ${indet ? "indeterminate" : ""}"><i style="width:${indet ? "" : pct + "%"}"></i></div>
    <div class="progress-meta"><span>${stageText}</span>
      <span>${j.stage === "downloading" ? `${fmtBytes(j.done)}${j.total ? " / " + fmtBytes(j.total) : ""}${j.speed ? " · " + fmtBytes(j.speed) + "/s" : ""}` : ""}</span></div>
  </div>`;
}

function installedList(t) {
  if (!t.installed.length)
    return `<div class="list">${emptyState("package", `还没有安装 ${t.tool}`, "从可安装列表中选择一个版本，或在“检测与接管”中接管系统已有的安装。",
      `<button class="btn btn-primary" data-tab="available" data-tool="${esc(t.tool)}">${icon("download", 14)}浏览可安装版本</button>`)}</div>`;
  const cur = t.current;
  return `<div class="list">${t.installed.map((iv) => {
    const isCur = cur?.version === iv.version;
    const isGlobal = t.global === iv.version;
    const isLocal = isCur && cur.spec.source.kind === "file";
    const size = usageOf(t.tool, iv.version);
    const adopted = iv.source === "adopted";
    const badges = [isCur ? `<span class="badge accent">当前</span>` : "", isGlobal ? `<span class="badge">全局</span>` : "",
      isLocal ? `<span class="badge">项目</span>` : "", adopted ? `<span class="badge warn" title="${esc(iv.path)}">${icon("link", 11)}已接管</span>` : "",
      adopted ? providerBadge(iv.provider, iv.formula) : "", iv.disabled ? `<span class="badge">已禁用</span>` : ""].join("");
    const meta = adopted ? `位于 ${shortPath(iv.path)}` : `${shortPath(iv.path)}${S.usage ? " · " + fmtBytes(size) : ""}`;
    return `<div class="item ${iv.disabled ? "is-disabled" : ""}">
      <span class="dot ${isCur ? "ok" : ""}"></span>
      <div class="main-col"><div class="title"><span class="ver">${esc(iv.version)}</span>${badges}</div><div class="meta mono">${esc(meta)}</div></div>
      <div class="actions">
        ${isGlobal || iv.disabled ? "" : `<button class="btn btn-sm" data-act="global" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}">设为全局</button>`}
        
        <button class="btn btn-sm" data-act="toggle" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}" data-disabled="${!!iv.disabled}" title="${iv.disabled ? "重新启用该版本" : "禁用后保留在列表中，但不会被使用"}">${iv.disabled ? "启用" : "禁用"}</button>
        ${iv.provider === "system" ? `<span class="badge" title="系统自带，禁止卸载，防止系统异常">禁止卸载</span><button class="btn btn-sm" data-act="uninstall" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}" data-adopted="${adopted}" data-system="true" title="只移除 runa 的管理，系统文件不变">移除管理</button>` : `<button class="btn btn-sm btn-icon btn-danger" data-act="uninstall" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}" data-adopted="${adopted}" data-system="${iv.provider === "system"}" aria-label="${iv.provider === "system" ? "移除管理" : "卸载"} ${esc(iv.version)}" title="${iv.provider === "system" ? "系统自带，禁止卸载（只能移除管理）" : adopted ? "卸载或移除管理" : "卸载"}">${icon("trash", 14)}</button>`}
      </div></div>`;
  }).join("")}</div>
  ${cur && cur.spec.source.kind === "file" && fileName(cur.spec.source.detail) === ".tool-versions" ? `<div class="hint-line" style="margin-top:12px">${icon("info", 13)}项目版本来自 <span class="mono">${esc(shortPath(cur.spec.source.detail))}</span><button class="btn btn-ghost btn-sm" data-act="unset-local" data-tool="${esc(t.tool)}">移除项目设置</button></div>` : ""}`;
}

function filterSeg(tool) {
  const r = S.remote[tool];
  const hasLts = r?.status === "ok" && r.items.some((x) => x.lts);
  const f = S.filter[tool] || "all";
  const opts = [["all", "全部"], ...(hasLts ? [["lts", "LTS"]] : []), ["stable", "稳定版"]];
  return `<div class="seg" role="radiogroup" aria-label="筛选">${opts.map(([k, l]) => `<button role="radio" aria-checked="${f === k}" data-filter="${k}" data-tool="${esc(tool)}">${l}</button>`).join("")}</div>`;
}

function availableList(t) {
  const tool = t.tool;
  const r = S.remote[tool] || { status: "loading" };
  const q = S.query[tool] || "";
  const busy = !!S.jobs[tool];
  const search = `<div class="toolbar">
      <div class="input-wrap"><span data-icon="search"></span><input class="input" id="q-${esc(tool)}" data-query="${esc(tool)}" placeholder="搜索或输入版本，如 20、3.12、latest、lts" value="${esc(q)}" autocomplete="off" spellcheck="false" /></div>
      <button class="btn btn-primary" data-act="install-query" data-tool="${esc(tool)}" ${busy ? "disabled" : ""}>${icon("download", 14)}安装</button>
      <button class="btn btn-icon" data-act="reload-remote" data-tool="${esc(tool)}" aria-label="刷新版本列表" title="刷新版本列表">${icon("refresh", 14, r.status === "loading" ? "spin" : "")}</button>
    </div>
    <div class="hint-line">${installerPicker(tool)} ${q ? `按 <span class="kbd">Enter</span> 安装匹配 “${esc(q)}” 的最新版本` : "支持模糊版本：输入主版本号即可安装该系列的最新版"}</div>`;
  if (r.status === "loading")
    return search + `<div class="list">${[1, 2, 3, 4, 5, 6].map(() => `<div class="item"><div class="main-col"><div class="skel" style="width:${60 + Math.random() * 60}px;height:12px"></div></div><div class="skel" style="width:56px;height:24px"></div></div>`).join("")}</div>`;
  if (r.status === "error")
    return search + `<div class="callout danger"><span data-icon="alert"></span><div class="grow"><div class="t">获取版本列表失败</div><div class="d">${esc(r.error)}</div></div>
      <button class="btn" data-act="reload-remote" data-tool="${esc(tool)}">重试</button></div>
      <div class="hint-line">网络受限时可在 <a href="#/settings" style="color:var(--accent-text)">设置</a> 中切换国内镜像或配置代理。</div>`;
  const f = S.filter[tool] || "all";
  const have = new Set(t.installed.map((i) => i.version));
  let items = r.items.filter((x) => (f === "lts" ? x.lts : f === "stable" ? !x.prerelease : true));
  if (q) items = items.filter((x) => x.version.startsWith(q.replace(/^v/, "")));
  const n = S.pageSize[tool] || 30;
  if (!items.length) return search + `<div class="list">${emptyState("search", "没有匹配的版本", q ? `没有以 “${q}” 开头的版本，换个关键词试试。` : "当前筛选条件下没有版本。")}</div>`;
  const shown = items.slice(0, n);
  return search + `<div class="list">${shown.map((x) => {
    const installed = have.has(x.version) || t.installed.some((iv) => tool === "java" && iv.version.split(".")[0] === x.version);
    return `<div class="item" style="min-height:48px">
      <div class="main-col"><div class="title"><span class="ver">${esc(x.version)}</span>${x.lts ? `<span class="badge accent">LTS</span>` : ""}${x.prerelease ? `<span class="badge">预览</span>` : ""}</div></div>
      ${installed ? `<span class="badge ok">${icon("check", 12)}已安装</span>` : `<div class="actions show"><button class="btn btn-sm btn-ghost" data-act="install" data-tool="${esc(tool)}" data-v="${esc(x.version)}" ${busy ? "disabled" : ""}>${icon("download", 13)}安装</button></div>`}
    </div>`;
  }).join("")}
  ${items.length > n ? `<div class="more"><button class="btn btn-ghost btn-sm" data-act="more" data-tool="${esc(tool)}">显示更多（剩余 ${items.length - n} 个）</button></div>` : ""}</div>`;
}

// ---------------------------------------------------------------- page: detect
function pageDetect() {
  const d = S.detected;
  let body;
  if (d.status === "idle" || (d.status === "loading" && !d.items.length))
    body = `<div class="list">${[1, 2, 3].map(() => `<div class="item"><span class="skel" style="width:32px;height:32px;border-radius:8px"></span><div class="main-col"><div class="skel" style="width:120px;height:11px"></div><div class="skel" style="width:220px;height:9px;margin-top:8px"></div></div></div>`).join("")}</div>`;
  else if (d.status === "error")
    body = `<div class="callout danger"><span data-icon="alert"></span><div class="grow"><div class="t">检测失败</div><div class="d">${esc(d.error)}</div></div><button class="btn" data-act="detect">重试</button></div>`;
  else if (!d.items.length) body = `<div class="list">${emptyState("checkCircle", "没有发现其它安装", "PATH、常见目录以及 nvm / pyenv / SDKMAN! / asdf / fnm / Homebrew 中都没有发现 Node、Java、Python 或 Go。")}</div>`;
  else
    body = `<div class="list">${d.items.map((x, i) => `<div class="item ${x.disabled ? "is-disabled" : ""} ${x.system_default ? "is-global" : ""}">${mark(x.tool, 32)}
      <div class="main-col"><div class="title">${esc(x.tool)} <span class="ver">${esc(x.version)}</span>${x.system_default ? `<span class="badge ok" title="系统 PATH 中排在最前，当前终端默认使用这个版本">${icon("check", 11)}当前全局</span>` : ""}${x.runa_global ? `<span class="badge">runa 全局</span>` : ""}${providerBadge(x.provider, x.formula)}</div><div class="meta mono" title="${esc(x.exe)}">${esc(shortPath(x.home))}</div></div>
      <span class="badge">${esc(x.via)}</span>
      ${x.view_only ? `<span class="badge" title="系统自带的运行时直接位于共享目录（如 /usr/bin），只能查看，不能接管或卸载">系统自带 · 仅查看</span>` : x.adopted ? `<span class="badge ok">${icon("check", 12)}已接管${x.adopted_as && x.adopted_as !== x.version ? ` · ${esc(x.adopted_as)}` : ""}</span>${x.disabled ? `<span class="badge">已禁用</span>` : ""}` : `<button class="btn btn-sm" data-act="adopt" data-i="${i}">${icon("link", 13)}接管</button>`}
    </div>`).join("")}</div>`;
  const mig = S.migration.length ? `<div class="callout warn"><span data-icon="alert"></span><div class="grow"><div class="t">有 ${S.migration.length} 个旧版接管记录</div>
      <div class="d">旧版本通过符号链接接管，迁移后改为登记真实安装位置：${S.migration.map((a) => `${esc(a.tool)} ${esc(a.version)}（${a.action === "convert" ? "转换" : "删除无效记录"}）`).join("、")}</div></div>
      <button class="btn" data-act="migrate">迁移</button></div>` : "";
  const meta = d.createdAt ? `<span class="detect-meta">${d.refreshing ? icon("refresh", 13, "spin") + "正在检查变化…" : `上次检测 ${ago(d.createdAt)}`}</span>` : "";
  return `<div class="page">
    <div class="page-head"><div><h1 class="page-title">检测与接管</h1><div class="page-sub">接管只登记真实安装位置，不创建链接，原有安装不会被移动或修改。</div></div>
    <div style="display:flex;gap:12px;align-items:center">${meta}${d.items.some((x) => !x.adopted && !x.view_only) ? `<button class="btn btn-primary" data-act="adopt-all" ${d.refreshing ? "disabled" : ""}>${icon("link", 14)}全部接管</button>` : ""}<button class="btn" data-act="detect" ${d.refreshing ? "disabled" : ""}>${icon("refresh", 14, d.refreshing ? "spin" : "")}重新检测</button></div></div>
    ${mig}${body}</div>`;
}

async function loadDetected() {
  if (S.detected.status === "idle") S.detected.status = "loading";
  try {
    const r = await invoke("detect_cached");
    S.detected = { ...S.detected, status: "ok", items: r.items, createdAt: r.created_at, error: null };
  } catch (e) { S.detected = { ...S.detected, status: "error", error: String(e) }; }
  if (route().name === "detect") render();
  checkDetect();
  invoke("migrate_adoptions", { dryRun: true }).then((a) => { S.migration = a; if (route().name === "detect") render(); }).catch(() => {});
}

let lastCheck = 0;
async function checkDetect(force = false) {
  if (!force && Date.now() - lastCheck < 30000) return;
  lastCheck = Date.now();
  S.detected.refreshing = true;
  if (route().name === "detect") render();
  try { await invoke("detect_check"); } catch (_) { /* best effort */ }
  S.detected.refreshing = false;
  if (route().name === "detect") render();
}

function onDetectUpdated(r) {
  S.detected = { ...S.detected, status: "ok", items: r.items, createdAt: r.created_at, error: null };
  if (route().name === "detect") render();
}

// ---------------------------------------------------------------- page: settings
function pageSettings() {
  const ov = S.ov, s = ov.settings;
  const proxyMode = S.proxyDraft?.mode ?? (s.proxy === "off" || s.proxy === "system" ? s.proxy : "custom");
  const proxyUrl = S.proxyDraft?.url ?? (proxyMode === "custom" ? s.proxy : "");
  const opt = (group, value, cur, title, desc) => `<label class="option ${cur === value ? "selected" : ""}" data-${group}="${value}">
      <input type="radio" name="${group}" value="${value}" ${cur === value ? "checked" : ""}/><span class="radio"></span>
      <div><div class="t">${title}</div><div class="d">${desc}</div></div></label>`;
  const shells = ov.shells.map((sh) => `<div class="item">
      <span data-icon="terminal" class="muted"></span>
      <div class="main-col"><div class="title">${esc(sh.shell)}${sh.shell === ov.default_shell ? `<span class="badge">默认</span>` : ""}</div><div class="meta mono">${esc(shortPath(sh.profile))}</div></div>
      ${sh.configured ? `<span class="badge ok">${icon("check", 12)}已启用</span>` : ""}
      <div class="actions show">${sh.configured ? `<button class="btn btn-sm btn-ghost" data-act="shell-remove" data-shell="${esc(sh.shell)}">移除</button><button class="btn btn-sm" data-act="shell-setup" data-shell="${esc(sh.shell)}">更新</button>`
        : `<button class="btn btn-sm" data-act="shell-setup" data-shell="${esc(sh.shell)}" ${ov.cli_found ? "" : "disabled"}>启用</button>`}</div></div>`).join("");
  return `<div class="page">
    <div class="page-head"><div><h1 class="page-title">设置</h1><div class="page-sub">命令行与桌面端共用同一份配置。</div></div></div>

    <section class="group"><div><h3>下载源</h3><p>选择运行时的下载来源。国内网络建议使用国内镜像。</p></div>
      <div class="options">
        ${opt("mirror", "off", s.mirror, "官方源", "nodejs.org · go.dev · Adoptium · GitHub")}
        ${opt("mirror", "cn", s.mirror, "国内镜像", "Node.js 与 Python 使用 npmmirror，Go 使用 golang.google.cn；Java 仍走 Adoptium")}
      </div></section>

    <section class="group"><div><h3>网络代理</h3><p>用于访问版本列表与下载。支持 http、https、socks5 与 socks5h。</p></div>
      <div>
        <div class="seg" role="radiogroup" aria-label="代理模式" style="margin-bottom:12px">
          ${[["system", "跟随系统"], ["off", "不使用"], ["custom", "自定义"]].map(([k, l]) => `<button role="radio" aria-checked="${proxyMode === k}" data-proxy-mode="${k}">${l}</button>`).join("")}
        </div>
        ${proxyMode === "custom" ? `<div class="toolbar" style="margin:0"><input class="input ${S.proxyError ? "invalid" : ""}" id="proxy-url" style="flex:1;font-family:var(--mono);font-size:12.5px" placeholder="http://127.0.0.1:7890" value="${esc(proxyUrl)}" spellcheck="false" />
          <button class="btn btn-primary" data-act="proxy-save">保存</button></div>
          ${S.proxyError ? `<div class="field-error">${icon("alert", 13)}${esc(S.proxyError)}</div>` : ""}`
        : `<p class="muted" style="font-size:12.5px">${proxyMode === "system" ? "读取 HTTPS_PROXY / HTTP_PROXY / ALL_PROXY / NO_PROXY 环境变量。" : "直接连接，不经过任何代理。"}</p>`}
      </div></section>

    <section class="group"><div><h3>Shell 激活</h3><p>在配置文件中写入带标记的代码块：把 shims 加入 PATH，并在每次提示符时按目录切换版本。修改前会自动备份。重开终端后生效。</p></div>
      <div><div class="list">${shells}</div>
        <div class="toolbar" style="margin-top:12px">
          <button class="btn btn-sm" data-act="reshim">${icon("refresh", 13)}重新生成 shims</button>
          ${ov.is_windows ? `<button class="btn btn-sm" data-act="user-path">把 shims 加入用户 PATH</button>` : ""}
        </div></div></section>

    <section class="group"><div><h3>外观</h3><p>默认跟随系统的浅色 / 深色设置。</p></div>
      <div><div class="seg" role="radiogroup" aria-label="主题">${THEMES.map(([k, ic, l]) => `<button role="radio" aria-checked="${themePref() === k}" data-theme-set="${k}">${icon(ic, 14)}${l}</button>`).join("")}</div></div></section>

    <section class="group"><div><h3>关于</h3><p>Runa ${esc(ov.app_version)}</p></div>
      <div class="kv"><span class="k">数据目录</span><span class="v">${esc(ov.home)}</span><span class="k">命令行</span><span class="v">${esc(ov.cli_path)}${ov.cli_found ? "" : "（未找到）"}</span><span class="k">平台</span><span class="v">${esc(ov.platform)}</span></div></section>
  </div>`;
}

// ---------------------------------------------------------------- actions
async function install(tool, spec) {
  if (S.jobs[tool]) return;
  S.jobs[tool] = { stage: "resolving", spec };
  render();
  try {
    const r = await invoke("install", { tool, spec });
    const v = r.version;
    toast(`${tool} ${v} 已安装`, r.via === "runa" ? "已生成 shims，可立即使用" : `已通过 ${r.via} 安装并登记，卸载时也会通过 ${r.via}`);
    S.tab[tool] = "installed";
  } catch (e) {
    toast(`安装 ${tool} ${spec} 失败`, String(e), "error");
  } finally {
    delete S.jobs[tool];
    await load({ quiet: true });
  }
}

function onProgress(p) {
  if (p.stage === "warning") { toast("注意", p.message, "error"); return; }
  const j = S.jobs[p.tool];
  if (!j) return;
  const now = performance.now();
  if (p.stage === "downloading") {
    if (j.lastT) {
      const dt = (now - j.lastT) / 1000;
      if (dt > 0.05) {
        const inst = (p.done - j.lastDone) / dt;
        j.speed = j.speed ? j.speed * 0.7 + inst * 0.3 : inst;
        j.lastT = now; j.lastDone = p.done;
      }
    } else { j.lastT = now; j.lastDone = p.done; }
    j.done = p.done; j.total = p.total;
  }
  j.stage = p.stage;
  if (p.version) j.version = p.version;
  if (p.via) { j.via = p.via; j.command = p.command; }
  if (p.stage === "output") j.line = p.line;
  // patch the progress card in place for smooth updates
  const card = document.querySelector(".progress-card");
  if (route().name === "tool" && route().arg === p.tool && card) {
    const tmp = document.createElement("div");
    tmp.innerHTML = jobCard(p.tool);
    card.replaceWith(tmp.firstElementChild);
  } else if (route().name === "tool" && route().arg === p.tool) render();
}

async function onAction(btn) {
  const { act, tool, v } = btn.dataset;
  switch (act) {
    case "reload": return load();
    case "install": return install(tool, v);
    case "install-query": { const q = (S.query[tool] || "").trim() || "latest"; return install(tool, q); }
    case "reload-remote": return loadRemote(tool, true);
    case "more": S.pageSize[tool] = (S.pageSize[tool] || 30) + 50; return render();
    case "global":
      await invoke("set_global", { tool, version: v }).catch(fail("设置失败"));
      toast(`${tool} ${v} 已设为全局版本`);
      return load({ quiet: true });
    case "local": {
      const f = await invoke("set_local", { project: projectDir(), tool, version: v }).catch(fail("设置失败"));
      toast(`已在项目中使用 ${tool} ${v}`, shortPath(f));
      return load({ quiet: true });
    }
    case "unset-local":
      await invoke("unset_local", { project: projectDir(), tool }).catch(fail("操作失败"));
      toast("已移除项目设置");
      return load({ quiet: true });
    case "uninstall": {
      const adopted = btn.dataset.adopted === "true";
      const system = btn.dataset.system === "true";
      const plans = {};
      const getPlan = async (mode) => (plans[mode] ??= await invoke("uninstall_plan", { tool, version: v, mode }));
      const mode = await choiceDialog(adopted ? {
        title: system ? `移除管理 ${tool} ${v}` : `卸载 ${tool} ${v}`,
        intro: system ? "系统自带，禁止卸载。只能移除 runa 的管理（或禁用该版本），系统文件保持不变。" : "这是接管的版本，请选择处理方式。",
        options: system ? [
          { key: "forget", title: "只移除管理", desc: "删除 runa 中的记录，原有安装和文件保持不变。" },
        ] : [
          { key: "forget", title: "只移除管理", desc: "删除 runa 中的记录，原有安装和文件保持不变。" },
          { key: "purge", title: "彻底卸载", desc: "通过它所属的工具（Homebrew、nvm、scoop、winget 等）卸载，或删除安装目录。" },
        ],
        initial: "forget",
        describe: async (m) => {
          let p;
          try { p = await getPlan(m); } catch (e) {
            return { html: `<p class="hint">${esc(String(e))}</p>`, okDisabled: true, danger: false, okLabel: "禁止卸载" };
          }
          const blocked = p.kind === "refuse" || p.needs_root;
          return { html: planHtml(p), okDisabled: p.kind === "refuse", danger: m === "purge", okLabel: m === "forget" ? "移除管理" : blocked ? "我已了解" : "彻底卸载" };
        },
      } : {
        title: `卸载 ${tool} ${v}？`,
        intro: "此操作无法撤销。",
        initial: "auto",
        describe: async () => ({ html: planHtml(await getPlan("auto")), danger: true, okLabel: "卸载" }),
      });
      if (!mode) return;
      const p = plans[mode];
      if (p?.needs_root) { toast("需要管理员权限", `请在终端执行：${p.command_line}`, "error"); return; }
      btn.disabled = true;
      await invoke("uninstall", { tool, version: v, mode }).catch(fail("卸载失败"));
      toast(mode === "forget" ? `已移除对 ${tool} ${v} 的管理` : `已卸载 ${tool} ${v}`, mode === "forget" ? "原有安装保持不变" : "");
      checkDetect(true);
      return load({ quiet: true });
    }
    case "toggle": {
      const disabled = btn.dataset.disabled !== "true";
      const warnings = await invoke("set_disabled", { tool, version: v, disabled, project: projectDir() || null }).catch(fail("操作失败"));
      if (warnings.length) warnings.forEach((w) => toast(`已禁用 ${tool} ${v}`, w, "error"));
      else toast(disabled ? `已禁用 ${tool} ${v}` : `已启用 ${tool} ${v}`, disabled ? "不会再被解析、生成 shim 或加入 PATH" : "");
      if (S.detected.items.length) S.detected.items = (await invoke("detect_cached").catch(() => null))?.items ?? S.detected.items;
      return load({ quiet: true });
    }
    case "detect": {
      S.detected = { ...S.detected, status: S.detected.items.length ? "ok" : "loading", refreshing: true }; render();
      try { const r = await invoke("detect_refresh"); S.detected = { status: "ok", items: r.items, createdAt: r.created_at, error: null, refreshing: false }; }
      catch (e) { S.detected = { ...S.detected, status: "error", error: String(e), refreshing: false }; }
      return render();
    }
    case "migrate": {
      const ok = await confirmDialog({ title: "迁移旧版接管记录？", body: "只会删除 runa 目录中的符号链接并改写记录，不会跟随链接，也不会修改原有安装。", ok: "迁移" });
      if (!ok) return;
      const acts = await invoke("migrate_adoptions", { dryRun: false }).catch(fail("迁移失败"));
      S.migration = [];
      toast(`已迁移 ${acts.length} 条记录`, acts.map((a) => `${a.tool} ${a.version}: ${a.note}`).join("；"));
      loadDetected();
      return load({ quiet: true });
    }
    case "adopt": {
      const item = S.detected.items[Number(btn.dataset.i)];
      btn.disabled = true;
      const res = await invoke("adopt", { item, name: null }).catch((e) => { btn.disabled = false; return fail("接管失败")(e); });
      if (!res) return;
      toast(`已接管 ${item.tool} ${res.version}`, res.set_global ? "这是当前全局使用的版本，已在 runa 中设为全局" : "可在版本列表中像普通版本一样切换");
      S.detected.items = (await invoke("detect_cached").catch(() => null))?.items ?? S.detected.items;
      return load({ quiet: true });
    }
    case "adopt-all": {
      const todo = S.detected.items.filter((x) => !x.adopted);
      if (!todo.length) return;
      const ok = await confirmDialog({ title: `接管全部 ${todo.length} 个版本？`, body: "只登记真实安装位置，不移动、不修改原有安装。标记为「当前全局」的版本会同时在 runa 中设为全局。版本号重复时会以「版本-来源」命名，例如 22.16.0-homebrew。", ok: "全部接管" });
      if (!ok) return;
      btn.disabled = true;
      const res = await invoke("adopt_all").catch((e) => { btn.disabled = false; return fail("接管失败")(e); });
      if (!res) return;
      const bad = res.filter((r) => r.error);
      const glob = res.filter((r) => r.set_global).map((r) => `${r.tool} ${r.adopted_as}`);
      toast(`已接管 ${res.length - bad.length} 个版本${bad.length ? `，${bad.length} 个失败` : ""}`,
        [glob.length ? `已设为全局：${glob.join("、")}` : "", bad.map((r) => `${r.tool} ${r.version}: ${r.error}`).join("；")].filter(Boolean).join("；") || "可在版本列表中像普通版本一样切换");
      S.detected.items = (await invoke("detect_cached").catch(() => null))?.items ?? S.detected.items;
      return load({ quiet: true });
    }
    case "fix-shell":
    case "shell-setup": {
      const shell = btn.dataset.shell || S.ov.default_shell;
      btn.disabled = true;
      const p = await invoke("setup_shell", { shell }).catch(fail("启用失败"));
      toast(`已为 ${shell} 启用自动切换`, `已写入 ${shortPath(p)}，重新打开终端后生效`);
      return load({ quiet: true });
    }
    case "shell-remove": {
      const shell = btn.dataset.shell;
      await invoke("remove_shell", { shell }).catch(fail("移除失败"));
      toast(`已从 ${shell} 配置中移除`);
      return load({ quiet: true });
    }
    case "reshim": { const n = await invoke("reshim").catch(fail("生成失败")); return toast(`已生成 ${n} 个 shim`); }
    case "user-path": await invoke("add_user_path").catch(fail("更新 PATH 失败")); return toast("已加入用户 PATH", "新打开的终端生效");
    case "proxy-save": return saveProxy("custom", $("#proxy-url").value.trim());
  }
}

async function saveProxy(mode, url) {
  const value = mode === "custom" ? url : mode;
  if (mode === "custom" && !/^(https?|socks5h?):\/\/\S+$/.test(url)) {
    S.proxyDraft = { mode, url };
    S.proxyError = "请输入以 http://、https://、socks5:// 或 socks5h:// 开头的地址";
    return render();
  }
  try {
    S.ov.settings = await invoke("set_proxy", { value });
    S.proxyDraft = null; S.proxyError = null;
    toast("代理设置已保存", mode === "custom" ? url : mode === "off" ? "不使用代理" : "跟随系统");
  } catch (e) { S.proxyError = String(e); }
  render();
}

// ---------------------------------------------------------------- events
document.addEventListener("click", (e) => {
  const th = e.target.closest("[data-theme-set]");
  if (th) { localStorage.setItem("runa.theme", th.dataset.themeSet); applyTheme(); if (route().name === "settings") render(); return; }
  const tab = e.target.closest("[data-tab]");
  if (tab) { S.tab[tab.dataset.tool] = tab.dataset.tab; return render(); }
  const f = e.target.closest("[data-filter]");
  if (f) { S.filter[f.dataset.tool] = f.dataset.filter; S.pageSize[f.dataset.tool] = 30; return render(); }
  const pm = e.target.closest("[data-proxy-mode]");
  if (pm) {
    const mode = pm.dataset.proxyMode;
    if (mode === "custom") { S.proxyDraft = { mode, url: S.proxyDraft?.url || "" }; S.proxyError = null; render(); $("#proxy-url")?.focus(); }
    else saveProxy(mode);
    return;
  }
  const mirror = e.target.closest("[data-mirror]");
  if (mirror) {
    e.preventDefault();
    const mode = mirror.dataset.mirror;
    if (mode === S.ov.settings.mirror) return;
    invoke("set_mirror", { mode }).then((st) => {
      S.ov.settings = st; S.remote = {};
      toast(mode === "cn" ? "已切换到国内镜像" : "已切换到官方源");
      render();
    }).catch(fail("切换失败"));
    return;
  }
  const btn = e.target.closest("button[data-act]");
  if (btn) onAction(btn).catch(() => {});
});

document.addEventListener("input", (e) => {
  const q = e.target.dataset?.query;
  if (q != null) { S.query[q] = e.target.value; S.pageSize[q] = 30; render(); }
  if (e.target.id === "proxy-url") { S.proxyDraft = { mode: "custom", url: e.target.value }; S.proxyError = null; e.target.classList.remove("invalid"); }
});
document.addEventListener("keydown", (e) => {
  if (e.key !== "Enter") return;
  const q = e.target.dataset?.query;
  if (q != null) install(q, e.target.value.trim() || "latest");
  if (e.target.id === "proxy-url") saveProxy("custom", e.target.value.trim());
  if (e.target.id === "project") e.target.blur();
});
$("#project").addEventListener("change", () => load({ quiet: true }));
document.addEventListener("change", async (e) => {
  const tool = e.target?.dataset?.installer;
  if (!tool) return;
  if (e.target.value.startsWith("install:")) {
    const name = e.target.value.slice(8);
    render(); // put the select back until the manager is installed
    return installManager(tool, name);
  }
  try { S.installer[tool] = await invoke("set_install_via", { tool, via: e.target.value }); }
  catch (err) { toast("无法设置安装方式", String(err), "error"); }
  render();
});
$("#refresh").addEventListener("click", () => { S.remote = {}; S.usage = null; load({ quiet: true }); });

// ---------------------------------------------------------------- boot
hydrateIcons();
applyTheme();
T.event.listen("install-progress", (ev) => onProgress(ev.payload));
T.event.listen("manager-progress", (ev) => onManagerProgress(ev.payload));
T.event.listen("detect-updated", (ev) => onDetectUpdated(ev.payload));
window.addEventListener("focus", () => { if (S.detected.status !== "idle") checkDetect(); });
render();
load();
loadDetected();
