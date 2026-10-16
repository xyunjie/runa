// All side effects of the UI: IPC calls, progress bookkeeping, toasts and confirmation flows.
import { toast } from "sonner";
import { api, type Detected, type ManagerAction, type ManagerPlan, type Progress, type UninstallMode, type UninstallPlan } from "./api";
import { getState, setIn, setState, useStore, type Job } from "./store";
import { providerName } from "./labels";
import { choiceDialog, confirmDialog } from "@/components/dialogs";
import { ManagerPlanView, PlanBox, UninstallPlanView } from "@/components/plan";

// ---------------------------------------------------------------- toasts
export function notify(title: string, detail = "", kind: "ok" | "error" = "ok") {
  if (kind === "error") toast.error(title, { description: detail || undefined, duration: 8000 });
  else toast.success(title, { description: detail || undefined, duration: 3200 });
}
const fail = (title: string) => (e: unknown): never => { notify(title, String(e), "error"); throw e; };

// ---------------------------------------------------------------- paths
export function shortPathWith(home: string | undefined, p: string): string {
  const h = home || "";
  if (!/[\\/]\.runa$/.test(h)) return p; // custom RUNA_HOME: show full paths
  const base = h.replace(/[\\/]\.runa$/, "");
  return p?.startsWith(base) ? "~" + p.slice(base.length) : p;
}
export const shortPath = (p: string) => shortPathWith(getState().ov?.home, p);
export function useShortPath() {
  const home = useStore((s) => s.ov?.home);
  return (p: string) => shortPathWith(home, p);
}
const projectDir = () => getState().project.trim();

// ---------------------------------------------------------------- data
export async function load({ quiet = false } = {}) {
  if (!quiet) setState((s) => ({ loading: !s.ov }));
  try {
    const ov = await api.overview(projectDir() || null);
    setState((s) => ({ ov, loadError: null, project: s.project || ov.project_dir }));
  } catch (e) {
    setState({ loadError: String(e) });
  }
  setState({ loading: false });
  api.diskUsage().then((usage) => setState({ usage })).catch(() => {});
}

export function refreshAll() {
  setState({ remote: {}, usage: null });
  return load({ quiet: true });
}

export async function loadRemote(tool: string, force = false) {
  if (!force && getState().remote[tool]?.status === "ok") return;
  setIn("remote", tool, { status: "loading" });
  try {
    setIn("remote", tool, { status: "ok", items: await api.remoteVersions(tool) });
  } catch (e) {
    setIn("remote", tool, { status: "error", error: String(e) });
  }
}

export async function loadInstaller(tool: string) {
  try { setIn("installer", tool, await api.installerChoice(tool)); } catch { /* optional */ }
}

export async function setInstaller(tool: string, via: string) {
  try { setIn("installer", tool, await api.setInstallVia(tool, via)); }
  catch (e) { notify("无法设置安装方式", String(e), "error"); }
}

// ---------------------------------------------------------------- install & progress
export async function install(tool: string, spec: string) {
  if (getState().jobs[tool]) return;
  setIn("jobs", tool, { stage: "resolving", spec });
  try {
    const r = await api.install(tool, spec);
    notify(`${tool} ${r.version} 已安装`, r.via === "runa" ? "已生成 shims，可立即使用" : `已通过 ${r.via} 安装并登记，卸载时也会通过 ${r.via}`);
    setIn("tab", tool, "installed");
  } catch (e) {
    notify(`安装 ${tool} ${spec} 失败`, String(e), "error");
  } finally {
    setIn("jobs", tool, undefined);
    await load({ quiet: true });
  }
}

export function onProgress(p: Progress) {
  if (p.stage === "warning") { notify("注意", p.message, "error"); return; }
  const prev = getState().jobs[p.tool];
  if (!prev) return;
  const j: Job = { ...prev };
  const now = performance.now();
  if (p.stage === "downloading") {
    if (j.lastT) {
      const dt = (now - j.lastT) / 1000;
      if (dt > 0.05) {
        const inst = (p.done - (j.lastDone || 0)) / dt;
        j.speed = j.speed ? j.speed * 0.7 + inst * 0.3 : inst;
        j.lastT = now; j.lastDone = p.done;
      }
    } else { j.lastT = now; j.lastDone = p.done; }
    j.done = p.done; j.total = p.total;
  }
  j.stage = p.stage;
  if ("version" in p && p.version) j.version = p.version;
  if (p.stage === "manager") { j.via = p.via; j.command = p.command; }
  if (p.stage === "output") j.line = p.line;
  setIn("jobs", p.tool, j);
}

export function onManagerLine(name: string, line: string) {
  const key = Object.keys(getState().jobs).find((t) => getState().jobs[t].mgr === name);
  if (key) onProgress({ stage: "output", tool: key, line });
}

function mgrInstallBody(plan: { display: string; rc_note?: string | null; note?: string | null }) {
  return (
    <>
      将执行以下命令（不会使用 sudo）：
      <code className="selectable my-2 block whitespace-pre-wrap font-mono text-[0.92em] text-fg">{plan.display}</code>
      {plan.rc_note ? <><b>会修改：</b>{plan.rc_note}</> : "不会修改 shell 配置文件。"}
      {plan.note ? <><br /><span className="text-fg-3">{plan.note}</span></> : null}
    </>
  );
}

/** From the 安装方式 picker: install a missing manager, then make it the installer of `tool`. */
export async function installManagerFor(tool: string, name: string) {
  let plan;
  try { plan = await api.managerInstallPlan(name); }
  catch (e) { notify(`无法安装 ${name}`, String(e), "error"); return; }
  const label = providerName(name);
  if (!(await confirmDialog({ title: `安装 ${label}`, body: mgrInstallBody(plan), ok: "安装" }))) return;
  if (getState().jobs[tool]) return;
  setIn("jobs", tool, { mgr: name, stage: "manager", via: label, command: plan.display });
  try {
    setIn("installer", tool, await api.installManager(name, tool));
    notify(`${label} 已安装`, `已设为 ${tool} 的安装方式，现在可以通过它安装新版本`);
  } catch (e) {
    notify(`安装 ${label} 失败`, String(e), "error");
  } finally {
    setIn("jobs", tool, undefined);
  }
}

// ---------------------------------------------------------------- managers page
export async function loadManagers() {
  try { setState({ mgrs: await api.managerInfo() }); } catch (e) { setState({ mgrs: { error: String(e) } }); }
}

export async function managerInstall(name: string) {
  let plan;
  try { plan = await api.managerInstallPlan(name); }
  catch (e) { notify(`无法安装 ${name}`, String(e), "error"); return; }
  const label = providerName(name);
  if (!(await confirmDialog({ title: `安装 ${label}`, body: mgrInstallBody(plan), ok: "安装" }))) return;
  const key = `mgr:${name}`;
  setIn("jobs", key, { mgr: name, title: `安装 ${label}`, stage: "manager", via: label, command: plan.display });
  try { await api.installManagerOnly(name); notify(`${label} 已安装`, "现在可以在运行时页面选择通过它安装新版本"); }
  catch (e) { notify(`安装 ${label} 失败`, String(e), "error"); }
  finally { setIn("jobs", key, undefined); setState({ installer: {} }); await loadManagers(); }
}

export async function managerAction(name: string, action: ManagerAction) {
  const verb = action === "upgrade" ? "升级" : "卸载";
  const label = providerName(name);
  let plan: ManagerPlan;
  try { plan = await api.managerPlan(name, action); }
  catch (e) { notify(`无法${verb} ${name}`, String(e), "error"); return; }
  if (plan.kind === "refuse") {
    await confirmDialog({ title: `${verb} ${label}`, body: plan.summary, ok: "知道了", cancel: false });
    return;
  }
  let keepData = false;
  if (action === "uninstall" && plan.data_runtimes.length) {
    // decision step: keep the runtimes the manager installed, or delete them with it
    const plans: Partial<Record<"keep" | "delete", ManagerPlan>> = { delete: plan };
    const pick = await choiceDialog<"keep" | "delete">({
      title: `卸载 ${label}`,
      intro: `${label} 的目录中还有 ${plan.data_runtimes.length} 个已安装的版本，请选择如何处理。`,
      options: [
        { value: "keep", title: "保留已安装的版本", desc: "只卸载管理器本身并移除 shell 配置行；版本目录保留，已登记的运行时改为“手动安装”。" },
        { value: "delete", title: "连同版本一起删除", desc: "删除管理器目录及其中的所有版本，并从 runa 取消登记。" },
      ],
      initial: "keep",
      describe: async (k) => {
        const p = (plans[k] ??= await api.managerPlan(name, action, k === "keep"));
        if (p.kind === "refuse") return { body: <PlanBox>{p.summary}</PlanBox>, okDisabled: true, okLabel: "卸载" };
        return { body: <ManagerPlanView p={p} />, danger: k === "delete", okLabel: k === "keep" ? "卸载管理器" : "全部删除" };
      },
    });
    if (!pick) return;
    keepData = pick === "keep";
    plan = plans[pick]!;
  } else if (!(await confirmDialog({ title: `${verb} ${label}`, body: <ManagerPlanView p={plan} />, ok: verb, danger: action === "uninstall" }))) {
    return;
  }
  const key = `mgr:${name}`;
  setIn("jobs", key, { mgr: name, title: `${verb} ${label}`, stage: "manager", via: label, command: plan.command_line || plan.summary });
  try {
    const rep = await api.managerExecute(name, action, keepData);
    notify(`已${verb} ${label}`, rep.notices.join("；") || (rep.unregistered.length ? `已取消登记 ${rep.unregistered.length} 个运行时` : ""));
  } catch (e) {
    notify(`${verb} ${label} 失败`, String(e), "error");
  } finally {
    setIn("jobs", key, undefined);
    await loadManagers();
    load({ quiet: true });
  }
}

// ---------------------------------------------------------------- installed versions
export async function setGlobal(tool: string, v: string) {
  await api.setGlobal(tool, v).catch(fail("设置失败"));
  notify(`${tool} ${v} 已设为全局版本`);
  return load({ quiet: true });
}

export async function unsetLocal(tool: string) {
  await api.unsetLocal(projectDir(), tool).catch(fail("操作失败"));
  notify("已移除项目设置");
  return load({ quiet: true });
}

export async function toggleDisabled(tool: string, v: string, disabled: boolean) {
  const warnings = await api.setDisabled(tool, v, disabled, projectDir() || null).catch(fail("操作失败"));
  if (warnings.length) warnings.forEach((w) => notify(`已禁用 ${tool} ${v}`, w, "error"));
  else notify(disabled ? `已禁用 ${tool} ${v}` : `已启用 ${tool} ${v}`, disabled ? "不会再被解析、生成 shim 或加入 PATH" : "");
  if (getState().detected.items.length) await refreshDetectedItems();
  return load({ quiet: true });
}

export async function uninstallFlow(tool: string, v: string, adopted: boolean, system: boolean) {
  const plans: Partial<Record<UninstallMode, UninstallPlan>> = {};
  const getPlan = async (mode: UninstallMode) => (plans[mode] ??= await api.uninstallPlan(tool, v, mode));
  const forgetOpt = { value: "forget" as const, title: "只移除管理", desc: "删除 runa 中的记录，原有安装和文件保持不变。" };
  const mode = await choiceDialog<UninstallMode>(adopted ? {
    title: system ? `移除管理 ${tool} ${v}` : `卸载 ${tool} ${v}`,
    intro: system ? "系统自带，禁止卸载。只能移除 runa 的管理（或禁用该版本），系统文件保持不变。" : "这是接管的版本，请选择处理方式。",
    options: system ? [forgetOpt] : [forgetOpt, { value: "purge", title: "彻底卸载", desc: "通过它所属的工具（Homebrew、nvm、scoop、winget 等）卸载，或删除安装目录。" }],
    initial: "forget",
    describe: async (m) => {
      let p: UninstallPlan;
      try { p = await getPlan(m); } catch (e) {
        return { body: <p className="mt-3 text-sm text-fg-3">{String(e)}</p>, okDisabled: true, danger: false, okLabel: "禁止卸载" };
      }
      const blocked = p.kind === "refuse" || p.needs_root;
      return { body: <UninstallPlanView p={p} />, okDisabled: p.kind === "refuse", danger: m === "purge", okLabel: m === "forget" ? "移除管理" : blocked ? "我已了解" : "彻底卸载" };
    },
  } : {
    title: `卸载 ${tool} ${v}？`,
    intro: "此操作无法撤销。",
    initial: "auto",
    describe: async () => ({ body: <UninstallPlanView p={await getPlan("auto")} />, danger: true, okLabel: "卸载" }),
  });
  if (!mode) return;
  const p = plans[mode];
  if (p?.needs_root) { notify("需要管理员权限", `请在终端执行：${p.command_line}`, "error"); return; }
  await api.uninstall(tool, v, mode).catch(fail("卸载失败"));
  notify(mode === "forget" ? `已移除对 ${tool} ${v} 的管理` : `已卸载 ${tool} ${v}`, mode === "forget" ? "原有安装保持不变" : "");
  checkDetect(true);
  return load({ quiet: true });
}

// ---------------------------------------------------------------- detection
async function refreshDetectedItems() {
  const r = await api.detectCached().catch(() => null);
  if (r) setState((s) => ({ detected: { ...s.detected, items: r.items } }));
}

export async function loadDetected() {
  setState((s) => ({ detected: { ...s.detected, status: s.detected.status === "idle" ? "loading" : s.detected.status } }));
  try {
    const r = await api.detectCached();
    setState((s) => ({ detected: { ...s.detected, status: "ok", items: r.items, createdAt: r.created_at, error: null } }));
  } catch (e) {
    setState((s) => ({ detected: { ...s.detected, status: "error", error: String(e) } }));
  }
  checkDetect();
  api.migrateAdoptions(true).then((migration) => setState({ migration })).catch(() => {});
}

let lastCheck = 0;
export async function checkDetect(force = false) {
  if (!force && Date.now() - lastCheck < 30000) return;
  lastCheck = Date.now();
  setState((s) => ({ detected: { ...s.detected, refreshing: true } }));
  try { await api.detectCheck(); } catch { /* best effort */ }
  setState((s) => ({ detected: { ...s.detected, refreshing: false } }));
}

export function onDetectUpdated(r: { items: Detected[]; created_at: number }) {
  setState((s) => ({ detected: { ...s.detected, status: "ok", items: r.items, createdAt: r.created_at, error: null } }));
}

export async function redetect() {
  setState((s) => ({ detected: { ...s.detected, status: s.detected.items.length ? "ok" : "loading", refreshing: true } }));
  try {
    const r = await api.detectRefresh();
    setState({ detected: { status: "ok", items: r.items, createdAt: r.created_at, error: null, refreshing: false } });
  } catch (e) {
    setState((s) => ({ detected: { ...s.detected, status: "error", error: String(e), refreshing: false } }));
  }
}

export async function adopt(item: Detected) {
  const res = await api.adopt(item, null).catch(fail("接管失败"));
  notify(`已接管 ${item.tool} ${res.version}`, res.set_global ? "这是当前全局使用的版本，已在 runa 中设为全局" : "可在版本列表中像普通版本一样切换");
  await refreshDetectedItems();
  return load({ quiet: true });
}

export async function adoptAll() {
  const todo = getState().detected.items.filter((x) => !x.adopted && !x.view_only);
  if (!todo.length) return;
  const ok = await confirmDialog({
    title: `接管全部 ${todo.length} 个版本？`,
    body: "只登记真实安装位置，不移动、不修改原有安装。标记为「当前全局」的版本会同时在 runa 中设为全局。版本号重复时会以「版本-来源」命名，例如 22.16.0-homebrew。",
    ok: "全部接管",
  });
  if (!ok) return;
  const res = await api.adoptAll().catch(fail("接管失败"));
  const bad = res.filter((r) => r.error);
  const glob = res.filter((r) => r.set_global).map((r) => `${r.tool} ${r.adopted_as}`);
  notify(
    `已接管 ${res.length - bad.length} 个版本${bad.length ? `，${bad.length} 个失败` : ""}`,
    [glob.length ? `已设为全局：${glob.join("、")}` : "", bad.map((r) => `${r.tool} ${r.version}: ${r.error}`).join("；")].filter(Boolean).join("；") || "可在版本列表中像普通版本一样切换",
  );
  await refreshDetectedItems();
  return load({ quiet: true });
}

export async function migrate() {
  const ok = await confirmDialog({ title: "迁移旧版接管记录？", body: "只会删除 runa 目录中的符号链接并改写记录，不会跟随链接，也不会修改原有安装。", ok: "迁移" });
  if (!ok) return;
  const acts = await api.migrateAdoptions(false).catch(fail("迁移失败"));
  setState({ migration: [] });
  notify(`已迁移 ${acts.length} 条记录`, acts.map((a) => `${a.tool} ${a.version}: ${a.note}`).join("；"));
  loadDetected();
  return load({ quiet: true });
}

// ---------------------------------------------------------------- settings
export async function setupShell(shell?: string) {
  const sh = shell || getState().ov!.default_shell;
  const p = await api.setupShell(sh).catch(fail("启用失败"));
  notify(`已为 ${sh} 启用自动切换`, `已写入 ${shortPath(p)}，重新打开终端后生效`);
  return load({ quiet: true });
}
export async function removeShell(shell: string) {
  await api.removeShell(shell).catch(fail("移除失败"));
  notify(`已从 ${shell} 配置中移除`);
  return load({ quiet: true });
}
export async function reshim() {
  const n = await api.reshim().catch(fail("生成失败"));
  notify(`已生成 ${n} 个 shim`);
}
export async function addUserPath() {
  await api.addUserPath().catch(fail("更新 PATH 失败"));
  notify("已加入用户 PATH", "新打开的终端生效");
}
export async function setMirror(mode: string) {
  if (mode === getState().ov?.settings.mirror) return;
  const st = await api.setMirror(mode).catch(fail("切换失败"));
  setState((s) => ({ ov: s.ov && { ...s.ov, settings: st }, remote: {} }));
  notify(mode === "cn" ? "已切换到国内镜像" : "已切换到官方源");
}
/** Returns an error message (shown under the field) or null. */
export async function saveProxy(mode: "system" | "off" | "custom", url = ""): Promise<string | null> {
  const value = mode === "custom" ? url : mode;
  if (mode === "custom" && !/^(https?|socks5h?):\/\/\S+$/.test(url)) return "请输入以 http://、https://、socks5:// 或 socks5h:// 开头的地址";
  try {
    const st = await api.setProxy(value);
    setState((s) => ({ ov: s.ov && { ...s.ov, settings: st } }));
    notify("代理设置已保存", mode === "custom" ? url : mode === "off" ? "不使用代理" : "跟随系统");
    return null;
  } catch (e) { return String(e); }
}


