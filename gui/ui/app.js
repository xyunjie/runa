// Runa desktop UI. Vanilla JS; talks to the Rust core over Tauri IPC.
import { icon, mark } from "./icons.js";

const T = window.__TAURI__;
const invoke = (cmd, args = {}) => T.core.invoke(cmd, args);

// ---------------------------------------------------------------- state
const S = {
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
  detected: { status: "idle", items: [], error: null },
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
  const stageText = { resolving: "正在解析版本…", downloading: "正在下载", verifying: "正在校验 SHA256…", extracting: "正在解压…", done: "完成" }[j.stage] || "准备中…";
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
    const badges = [isCur ? `<span class="badge accent">当前</span>` : "", isGlobal ? `<span class="badge">全局</span>` : "",
      isLocal ? `<span class="badge">项目</span>` : "", iv.source === "adopted" ? `<span class="badge warn" title="${esc(iv.origin)}">${icon("link", 11)}已接管</span>` : ""].join("");
    const meta = iv.source === "adopted" ? `链接到 ${shortPath(iv.origin)}` : `${shortPath(iv.path)}${S.usage ? " · " + fmtBytes(size) : ""}`;
    return `<div class="item">
      <span class="dot ${isCur ? "ok" : ""}"></span>
      <div class="main-col"><div class="title"><span class="ver">${esc(iv.version)}</span>${badges}</div><div class="meta mono">${esc(meta)}</div></div>
      <div class="actions">
        ${isGlobal ? "" : `<button class="btn btn-sm" data-act="global" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}">设为全局</button>`}
        <button class="btn btn-sm" data-act="local" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}" title="写入 ${esc(projectDir())}/.tool-versions">用于项目</button>
        <button class="btn btn-sm btn-icon btn-danger" data-act="uninstall" data-tool="${esc(t.tool)}" data-v="${esc(iv.version)}" data-adopted="${iv.source === "adopted"}" aria-label="${iv.source === "adopted" ? "取消接管" : "卸载"} ${esc(iv.version)}" title="${iv.source === "adopted" ? "取消接管" : "卸载"}">${icon("trash", 14)}</button>
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
    <div class="hint-line">${q ? `按 <span class="kbd">Enter</span> 安装匹配 “${esc(q)}” 的最新版本` : "支持模糊版本：输入主版本号即可安装该系列的最新版"}</div>`;
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
  if (d.status === "idle")
    body = `<div class="list">${emptyState("scan", "扫描本机已有的运行时", "检查 PATH、JAVA_HOME、GOROOT 以及常见安装目录。",
      `<button class="btn btn-primary" data-act="detect">${icon("scan", 14)}开始检测</button>`)}</div>`;
  else if (d.status === "loading")
    body = `<div class="list">${[1, 2, 3].map(() => `<div class="item"><span class="skel" style="width:32px;height:32px;border-radius:8px"></span><div class="main-col"><div class="skel" style="width:120px;height:11px"></div><div class="skel" style="width:220px;height:9px;margin-top:8px"></div></div></div>`).join("")}</div>`;
  else if (d.status === "error")
    body = `<div class="callout danger"><span data-icon="alert"></span><div class="grow"><div class="t">检测失败</div><div class="d">${esc(d.error)}</div></div><button class="btn" data-act="detect">重试</button></div>`;
  else if (!d.items.length) body = `<div class="list">${emptyState("checkCircle", "没有发现其它安装", "本机 PATH 中没有可接管的 Node、Java、Python 或 Go。")}</div>`;
  else
    body = `<div class="list">${d.items.map((x, i) => `<div class="item">${mark(x.tool, 32)}
      <div class="main-col"><div class="title">${esc(x.tool)} <span class="ver">${esc(x.version)}</span></div><div class="meta mono">${esc(x.exe)}</div></div>
      <span class="badge">${esc(x.via)}</span>
      ${x.adopted ? `<span class="badge ok">${icon("check", 12)}已接管</span>` : `<button class="btn btn-sm" data-act="adopt" data-i="${i}">${icon("link", 13)}接管</button>`}
    </div>`).join("")}</div>`;
  return `<div class="page">
    <div class="page-head"><div><h1 class="page-title">检测与接管</h1><div class="page-sub">接管只会创建符号链接，原有安装不会被移动或修改。</div></div>
    ${d.status === "ok" || d.status === "error" ? `<button class="btn" data-act="detect">${icon("refresh", 14)}重新检测</button>` : ""}</div>
    ${body}</div>`;
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
    const v = await invoke("install", { tool, spec });
    toast(`${tool} ${v} 已安装`, "已生成 shims，可立即使用");
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
      const ok = await confirmDialog({
        title: adopted ? `取消接管 ${tool} ${v}？` : `卸载 ${tool} ${v}？`,
        body: adopted ? "只会删除 runa 中的链接，系统原有安装保持不变。" : `将删除 <span class="mono">${esc(shortPath(S.ov.home + "/installs/" + tool + "/" + v))}</span>，此操作无法撤销。`,
        ok: adopted ? "取消接管" : "卸载", danger: true,
      });
      if (!ok) return;
      await invoke("uninstall", { tool, version: v }).catch(fail("卸载失败"));
      toast(adopted ? `已取消接管 ${tool} ${v}` : `已卸载 ${tool} ${v}`);
      return load({ quiet: true });
    }
    case "detect":
      S.detected = { status: "loading", items: [] }; render();
      try { S.detected = { status: "ok", items: await invoke("detect_runtimes") }; }
      catch (e) { S.detected = { status: "error", items: [], error: String(e) }; }
      return render();
    case "adopt": {
      const item = S.detected.items[Number(btn.dataset.i)];
      btn.disabled = true;
      const ver = await invoke("adopt", { item, name: null }).catch(fail("接管失败"));
      toast(`已接管 ${item.tool} ${ver}`, "可在版本列表中像普通版本一样切换");
      S.detected.items = await invoke("detect_runtimes").catch(() => S.detected.items);
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
$("#refresh").addEventListener("click", () => { S.remote = {}; S.usage = null; load({ quiet: true }); });

// ---------------------------------------------------------------- boot
hydrateIcons();
applyTheme();
T.event.listen("install-progress", (ev) => onProgress(ev.payload));
render();
load();
