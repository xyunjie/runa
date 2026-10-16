import { useEffect, useState } from "react";
import { AlertTriangle, Check, Download, FolderGit2, FolderOpen, FolderPlus, RefreshCw, ScanSearch, Trash2, Wand2 } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { List, Row } from "@/components/ui/card";
import { Callout, EmptyState, Mark, Page, PageHead } from "@/components/common";
import { ProgressCard } from "@/components/progress-card";
import { api, inTauri, type ProjectDetail, type ProjectItem } from "@/lib/api";
import { install, notify, useShortPath } from "@/lib/actions";
import { getState, setState, useStore } from "@/lib/store";

export async function loadProjects() {
  try { setState({ projects: (await api.projectsList()) || [] }); } catch { /* ignore */ }
}

export async function addProjects(paths: string[], go = true) {
  if (!paths.length) return;
  try {
    const before = new Set(getState().projects);
    const list = await api.projectAdd(paths);
    setState({ projects: list });
    const added = list.filter((p) => !before.has(p));
    notify(added.length ? `已添加 ${added.length} 个项目` : "项目已在列表中");
    const target = added[0] || list.find((p) => paths.includes(p));
    if (go && target) location.hash = `#/project/${encodeURIComponent(target)}`;
  } catch (e) { notify("添加项目失败", String(e), "error"); }
}

/** Added folders that are not projects themselves but contain sub-projects open a picker. */
export async function addFolders(paths: string[], go = true) {
  const direct: string[] = [];
  for (const p of paths) {
    let isProj = true;
    try { isProj = await api.projectIsProject(p); } catch { /* treat as project */ }
    if (isProj) { direct.push(p); continue; }
    let found: string[] = [];
    try { found = (await api.projectScan(p)) || []; } catch { /* ignore */ }
    if (found.length) setState({ subPick: { root: p, found } });
    else direct.push(p);
  }
  if (direct.length) await addProjects(direct, go && !getState().subPick);
}

function SubProjectPicker() {
  const pick = useStore((s) => s.subPick);
  const projects = useStore((s) => s.projects);
  const sp = useShortPath();
  const [sel, setSel] = useState<Set<string>>(new Set());
  useEffect(() => { setSel(new Set(pick?.found || [])); }, [pick]);
  if (!pick) return null;
  const close = () => setState({ subPick: null });
  const rel = (f: string) => f.startsWith(pick.root) ? f.slice(pick.root.length).replace(/^[\\/]/, "") || f : f;
  const toggle = (f: string) => setSel((s) => { const n = new Set(s); n.has(f) ? n.delete(f) : n.add(f); return n; });
  return (
    <Dialog open onOpenChange={(o) => { if (!o) close(); }}>
      <DialogContent wide>
        <DialogTitle>发现 {pick.found.length} 个子项目</DialogTitle>
        <DialogDescription>{sp(pick.root)} 本身不是项目，但其中包含以下项目。选择要添加的项目（多模块 Maven / Gradle 工程按一个项目添加）。</DialogDescription>
        <div className="mt-3 flex items-center gap-2 text-sm text-fg-2">
          <button className="underline" onClick={() => setSel(new Set(pick.found))}>全选</button>
          <button className="underline" onClick={() => setSel(new Set())}>全不选</button>
          <span className="ml-auto">已选 {sel.size} 个</span>
        </div>
        <div className="mt-2 max-h-[320px] overflow-y-auto rounded-lg border border-border">
          {pick.found.map((f) => (
            <label key={f} className="flex cursor-pointer items-center gap-2 border-b border-border px-3 py-2 text-sm last:border-b-0" title={f}>
              <input type="checkbox" checked={sel.has(f)} onChange={() => toggle(f)} />
              <span className="min-w-0 truncate">{rel(f)}</span>
              {projects.includes(f) && <Badge>已添加</Badge>}
            </label>
          ))}
        </div>
        <DialogFooter>
          <Button size="sm" onClick={() => { const r = pick.root; close(); addProjects([r]); }}>仅添加此文件夹</Button>
          <Button size="sm" onClick={close}>取消</Button>
          <Button size="sm" variant="primary" disabled={!sel.size} onClick={() => { const l = pick.found.filter((f) => sel.has(f)); close(); addProjects(l, false); }}>添加选中的 {sel.size} 个项目</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

async function pickFolder(multiple = true): Promise<string[]> {
  if (!inTauri) return ["/home/dev/code/new-app"];
  const r = await open({ directory: true, multiple, title: "选择项目文件夹" });
  return r == null ? [] : Array.isArray(r) ? r : [r];
}

/** Folders dropped onto the window are added as projects. */
function useDrop(onPaths: (p: string[]) => void) {
  const [over, setOver] = useState(false);
  useEffect(() => {
    if (!inTauri) return;
    const un = getCurrentWebview().onDragDropEvent((e) => {
      const t = e.payload.type;
      if (t === "enter" || t === "over") setOver(true);
      else if (t === "leave") setOver(false);
      else if (t === "drop") { setOver(false); onPaths(e.payload.paths); }
    });
    return () => { un.then((f) => f()); };
  }, []);
  return over;
}

export function ProjectsPage() {
  const projects = useStore((s) => s.projects);
  const sp = useShortPath();
  const [scan, setScan] = useState<{ root: string; found: string[] } | null>(null);
  const [scanning, setScanning] = useState(false);
  const over = useDrop((p) => addFolders(p));
  const doScan = async () => {
    const [root] = await pickFolder(false);
    if (!root) return;
    setScanning(true);
    try { setScan({ root, found: await api.projectScan(root) }); }
    catch (e) { notify("扫描失败", String(e), "error"); }
    setScanning(false);
  };
  const remove = async (p: string) => {
    try { setState({ projects: await api.projectRemove(p) }); } catch (e) { notify("移除失败", String(e), "error"); }
  };
  return (
    <Page>
      <SubProjectPicker />
      <PageHead title="项目" sub="按项目中的版本文件自动切换运行时版本；在这里查看并补齐每个项目需要的版本。">
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={scanning} onClick={doScan}><ScanSearch size={13} />{scanning ? "扫描中…" : "扫描文件夹"}</Button>
          <Button size="sm" variant="primary" onClick={async () => addFolders(await pickFolder())}><FolderPlus size={13} />添加项目</Button>
        </div>
      </PageHead>
      <div className={"mb-6 rounded-xl border border-dashed px-4 py-6 text-center text-sm transition-colors " + (over ? "border-accent bg-accent-soft text-fg" : "border-border text-fg-3")}>
        <FolderOpen size={18} className="mx-auto mb-1.5" />把项目文件夹拖到窗口中即可添加
      </div>
      {scan && (
        <div className="mb-6">
          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
            <div className="min-w-0 truncate text-sm text-fg-2" title={scan.root}>在 {sp(scan.root)} 中找到 {scan.found.length} 个声明了版本的文件夹</div>
            <div className="flex gap-2">
              <Button size="sm" variant="ghost" onClick={() => setScan(null)}>关闭</Button>
              <Button size="sm" variant="primary" disabled={!scan.found.length} onClick={() => { addProjects(scan.found.filter((f) => !projects.includes(f)), false); setScan(null); }}>全部添加</Button>
            </div>
          </div>
          {scan.found.length > 0 && (
            <List>
              {scan.found.map((f) => (
                <Row key={f} className="min-h-11">
                  <div className="min-w-0 flex-1 truncate font-mono text-sm" title={f}>{sp(f)}</div>
                  <div data-actions="" data-show="">
                    {projects.includes(f) ? <Badge variant="ok"><Check size={12} />已添加</Badge>
                      : <Button size="sm" onClick={() => addProjects([f], false)}>添加</Button>}
                  </div>
                </Row>
              ))}
            </List>
          )}
        </div>
      )}
      {!projects.length ? (
        <List><EmptyState icon={FolderGit2} title="还没有项目" desc="添加一个包含 .tool-versions、.nvmrc、.python-version、go.mod 等版本文件的文件夹。" /></List>
      ) : (
        <List>
          {projects.map((p) => (
            <Row key={p}>
              <FolderGit2 size={18} strokeWidth={1.75} className="flex-none text-fg-3" />
              <a className="min-w-0 flex-1" href={`#/project/${encodeURIComponent(p)}`}>
                <div className="font-medium">{p.split(/[\\/]/).filter(Boolean).pop()}</div>
                <div className="truncate font-mono text-sm text-fg-3" title={p}>{sp(p)}</div>
              </a>
              <div data-actions="">
                <Button size="sm" variant="ghost" onClick={() => remove(p)}><Trash2 size={13} />移除</Button>
              </div>
            </Row>
          ))}
        </List>
      )}
    </Page>
  );
}

const STATUS: Record<string, { label: string; variant: "ok" | "warn" | "default" }> = {
  ok: { label: "已满足", variant: "ok" },
  mismatch: { label: "版本不一致", variant: "warn" },
  missing: { label: "未安装", variant: "warn" },
  system: { label: "使用系统版本", variant: "default" },
};

function ItemRow({ it, path, onChanged }: { it: ProjectItem; path: string; onChanged: () => void }) {
  const sp = useShortPath();
  const job = useStore((s) => s.jobs[it.tool]);
  const st = STATUS[it.status];
  const setVersion = async (v: string) => {
    if (!v || v === it.resolved) return;
    try { const f = await api.projectSetVersion(path, it.tool, v); notify(`${it.tool} 已改为 ${v}`, `写入 ${sp(f)}`); }
    catch (e) { notify("修改版本失败", String(e), "error"); }
    onChanged();
  };
  return (
    <Row>
      <Mark tool={it.tool} size={32} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2 font-medium">
          {it.tool}<span className="font-mono">{it.required}</span>
          <Badge variant={st.variant}>{it.status === "ok" ? <Check size={11} /> : it.status !== "system" ? <AlertTriangle size={11} /> : null}{st.label}</Badge>
          {it.resolved && it.resolved !== it.required && <span className="font-mono text-sm text-fg-3">→ {it.resolved}</span>}
        </div>
        <div className="selectable truncate font-mono text-sm text-fg-3" title={it.source}>{sp(it.source)}</div>
      </div>
      <div data-actions="" data-show="">
        {it.installed.length > 0 && (
          <select aria-label={`修改 ${it.tool} 版本`} value={it.resolved || ""} onChange={(e) => setVersion(e.target.value)}
            className="h-[26px] max-w-[160px] rounded-md border border-border bg-surface px-1.5 font-mono text-xs text-fg shadow-xs outline-none focus-visible:outline-2 focus-visible:outline-accent">
            {!it.resolved && <option value="">改用已安装版本…</option>}
            {it.installed.map((v) => <option key={v} value={v}>{v}</option>)}
          </select>
        )}
        {(it.status === "mismatch" || it.status === "missing") && (
          <Button size="sm" disabled={!!job} onClick={() => install(it.tool, it.required).then(onChanged)}><Download size={13} />安装 {it.required}</Button>
        )}
      </div>
    </Row>
  );
}

export function ProjectPage({ path }: { path: string }) {
  const sp = useShortPath();
  const [d, setD] = useState<ProjectDetail | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [filling, setFilling] = useState(false);
  const refresh = () => api.projectDetail(path).then((x) => { setD(x); setErr(null); }).catch((e) => setErr(String(e)));
  useEffect(() => { refresh(); const f = () => refresh(); window.addEventListener("focus", f); return () => window.removeEventListener("focus", f); }, [path]);
  if (err) return <Page><PageHead title="项目" /><Callout kind="danger" title="无法读取项目">{err}</Callout></Page>;
  if (!d) return <Page><PageHead title="项目" sub="加载中…" /></Page>;
  const missing = d.items.filter((i) => i.status === "mismatch" || i.status === "missing");
  const fill = async () => {
    setFilling(true);
    for (const it of missing) await install(it.tool, it.required);
    setFilling(false);
    refresh();
  };
  return (
    <Page>
      <PageHead title={d.name} sub={<span className="selectable break-all font-mono text-sm" title={d.path}>{sp(d.path)}</span>}>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={refresh}><RefreshCw size={13} />刷新</Button>
          <Button size="sm" variant="primary" disabled={!missing.length || filling} onClick={fill}><Wand2 size={13} />{filling ? "补齐中…" : `一键补齐${missing.length ? `（${missing.length}）` : ""}`}</Button>
        </div>
      </PageHead>
      {!d.exists && <Callout kind="danger" title="文件夹不存在">项目可能已被移动或删除，可在“管理项目”中移除。</Callout>}
      {d.items.map((it) => <ProgressCard key={`p-${it.tool}`} id={it.tool} />)}
      {d.exists && !d.items.length ? (
        <List><EmptyState icon={FolderGit2} title="没有发现版本要求" desc="这个文件夹及其上级目录中没有 .tool-versions、.nvmrc、.python-version、go.mod 等版本文件。用 runa use <工具> <版本> 或下方任一运行时页面设置。" /></List>
      ) : (
        <List>{d.items.map((it) => <ItemRow key={it.tool} it={it} path={d.path} onChanged={refresh} />)}</List>
      )}
      <p className="mt-3 text-xs text-fg-3">在此项目目录下运行 node、python、java 等命令时，runa 的 shims 会自动使用上面的版本；修改版本会写入对应的版本文件（无法原样改写的文件如 package.json、go.mod 会在同目录写入 .tool-versions）。</p>
    </Page>
  );
}
