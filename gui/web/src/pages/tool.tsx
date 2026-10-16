import { useEffect, useRef } from "react";
import { Check, Download, Info, Link2, Package, RefreshCw, Search, Trash2 } from "lucide-react";
import type { ToolStatus } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { List, Row } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Callout, Dot, EmptyState, Mark, Page } from "@/components/common";
import { ProgressCard } from "@/components/progress-card";
import { InstallerPicker } from "@/components/installer-picker";
import { setIn, useStore } from "@/lib/store";
import { install, installManagerFor, loadInstaller, loadRemote, setGlobal, toggleDisabled, uninstallFlow, unsetLocal, useShortPath } from "@/lib/actions";
import { fileName, fmtBytes } from "@/lib/utils";
import { providerName } from "@/lib/labels";
import { ProviderBadge, SourceChip, usageOf } from "./helpers";

export function ToolPage({ name }: { name: string }) {
  const t = useStore((s) => s.ov!.tools.find((x) => x.tool === name));
  const tabSel = useStore((s) => s.tab[name]);
  const busy = useStore((s) => !!s.jobs[name]);
  const hasRemote = useStore((s) => !!s.remote[name]);
  const hasInstaller = useStore((s) => !!s.installer[name]);
  const tab = tabSel || (t?.installed.length ? "installed" : "available");
  useEffect(() => {
    if (tab === "available" && !hasRemote) loadRemote(name);
    if (tab === "available" && !hasInstaller) loadInstaller(name);
  }, [tab, name, hasRemote, hasInstaller]);
  if (!t) return <Page><EmptyState icon={Package} title="未知工具" desc={name} /></Page>;
  const cur = t.current;
  return (
    <Page key={name}>
      <div className="flex items-center gap-4">
        <Mark tool={name} size={48} />
        <div className="min-w-0 flex-1">
          <h1 className="m-0 text-xl font-semibold leading-tight tracking-[-0.02em]">{name}</h1>
          <div className="mt-1.5 text-fg-3">{t.description}{t.builtin ? "" : " · JSON 插件"}</div>
        </div>
        <div className="text-right">
          {!cur ? (<><div className="mb-0.5 text-xs text-fg-3">当前版本</div><div className="text-fg-3">未设置</div></>)
            : cur.system || cur.version ? (
              <><div className="mb-0.5 text-xs text-fg-3">当前版本</div><div className="font-mono text-lg font-medium tracking-[-0.02em]">{cur.system ? "system" : cur.version}</div>
                <div className="mt-1.5 flex justify-end"><SourceChip cur={cur} /></div></>
            ) : (
              <><div className="mb-0.5 text-xs text-fg-3">需要但未安装</div><div className="font-mono text-lg font-medium text-warn">{cur.spec.specs[0]}</div>
                <div className="mt-1.5 flex justify-end gap-1.5"><SourceChip cur={cur} />
                  <Button variant="primary" size="sm" disabled={busy} onClick={() => install(name, cur.spec.specs[0])}><Download size={13} />安装</Button></div></>
            )}
        </div>
      </div>
      <ProgressCard id={name} />
      <Tabs value={tab} onValueChange={(v) => setIn("tab", name, v as "installed" | "available")}>
        <div className="mb-4 mt-8 flex items-center justify-between gap-3">
          <TabsList>
            <TabsTrigger value="installed">已安装 <span className="font-mono text-xs text-fg-3">{t.installed.length}</span></TabsTrigger>
            <TabsTrigger value="available">可安装</TabsTrigger>
          </TabsList>
          {tab === "available" && <FilterSeg tool={name} />}
        </div>
      </Tabs>
      {tab === "installed" ? <InstalledList t={t} /> : <AvailableList t={t} />}
    </Page>
  );
}

function InstalledList({ t }: { t: ToolStatus }) {
  const usage = useStore((s) => s.usage);
  const sp = useShortPath();
  if (!t.installed.length)
    return (
      <List>
        <EmptyState icon={Package} title={`还没有安装 ${t.tool}`} desc="从可安装列表中选择一个版本，或在“检测与接管”中接管系统已有的安装。"
          action={<Button variant="primary" onClick={() => setIn("tab", t.tool, "available")}><Download size={14} />浏览可安装版本</Button>} />
      </List>
    );
  const cur = t.current;
  const projFile = cur && cur.spec.source.kind === "file" && fileName(cur.spec.source.detail) === ".tool-versions" ? cur.spec.source.detail : null;
  return (
    <>
      <List>
        {t.installed.map((iv) => {
          const isCur = cur?.version === iv.version;
          const isGlobal = t.global === iv.version;
          const isLocal = isCur && cur!.spec.source.kind === "file";
          const adopted = iv.source === "adopted";
          const system = iv.provider === "system";
          const size = usageOf(usage, t.tool, iv.version);
          const meta = adopted ? `位于 ${sp(iv.path)}` : `${sp(iv.path)}${usage ? " · " + fmtBytes(size) : ""}`;
          return (
            <Row key={iv.version}>
              <Dot status={isCur ? "ok" : ""} />
              <div className={iv.disabled ? "min-w-0 flex-1 opacity-55" : "min-w-0 flex-1"}>
                <div className="flex items-center gap-2 font-medium">
                  <span className="font-mono">{iv.version}</span>
                  {isCur && <Badge variant="accent">当前</Badge>}
                  {isGlobal && <Badge>全局</Badge>}
                  {isLocal && <Badge>项目</Badge>}
                  {adopted && <Badge variant="warn" title={iv.path}><Link2 size={11} />已接管</Badge>}
                  {adopted && <ProviderBadge provider={iv.provider} formula={iv.formula} />}
                  {iv.disabled && <Badge>已禁用</Badge>}
                </div>
                <div className="selectable truncate font-mono text-sm text-fg-3">{meta}</div>
              </div>
              <div data-actions="">
                {!isGlobal && !iv.disabled && <Button size="sm" onClick={() => setGlobal(t.tool, iv.version).catch(() => {})}>设为全局</Button>}
                <Button size="sm" title={iv.disabled ? "重新启用该版本" : "禁用后保留在列表中，但不会被使用"}
                  onClick={() => toggleDisabled(t.tool, iv.version, !iv.disabled).catch(() => {})}>{iv.disabled ? "启用" : "禁用"}</Button>
                {system ? (
                  <>
                    <Badge title="系统自带，禁止卸载，防止系统异常" className="self-center">禁止卸载</Badge>
                    <Button size="sm" title="只移除 runa 的管理，系统文件不变" onClick={() => uninstallFlow(t.tool, iv.version, adopted, true).catch(() => {})}>移除管理</Button>
                  </>
                ) : (
                  <Button size="icon-sm" variant="danger" aria-label={`卸载 ${iv.version}`} title={adopted ? "卸载或移除管理" : "卸载"}
                    onClick={() => uninstallFlow(t.tool, iv.version, adopted, false).catch(() => {})}><Trash2 size={14} /></Button>
                )}
              </div>
            </Row>
          );
        })}
      </List>
      {projFile && (
        <div className="mt-3 flex min-h-[18px] items-center gap-1.5 text-sm text-fg-3">
          <Info size={13} />项目版本来自 <span className="font-mono">{sp(projFile)}</span>
          <Button variant="ghost" size="sm" onClick={() => unsetLocal(t.tool).catch(() => {})}>移除项目设置</Button>
        </div>
      )}
    </>
  );
}

function FilterSeg({ tool }: { tool: string }) {
  const r = useStore((s) => s.remote[tool]);
  const f = useStore((s) => s.filter[tool] || "all");
  const hasLts = r?.status === "ok" && r.items.some((x) => x.lts);
  const items = [{ value: "all" as const, label: "全部" }, ...(hasLts ? [{ value: "lts" as const, label: "LTS" }] : []), { value: "stable" as const, label: "稳定版" }];
  return <Segmented label="筛选" value={f} items={items} onChange={(v) => { setIn("filter", tool, v); setIn("pageSize", tool, 30); }} />;
}

function AvailableList({ t }: { t: ToolStatus }) {
  const tool = t.tool;
  const r = useStore((s) => s.remote[tool]) || { status: "loading" as const };
  const q = useStore((s) => s.query[tool] || "");
  const f = useStore((s) => s.filter[tool] || "all");
  const n = useStore((s) => s.pageSize[tool] || 30);
  const busy = useStore((s) => !!s.jobs[tool]);
  const choice = useStore((s) => s.installer[tool]);
  const inputRef = useRef<HTMLInputElement>(null);
  // Ruby (and Rust on Windows) have no portable official binary: a manager is required
  const runaOpt = choice?.options.find((o) => o.via === "runa");
  const mgrs = choice?.options.filter((o) => o.via !== "runa") || [];
  const needManager = !!runaOpt && !runaOpt.available && !mgrs.some((o) => o.available);
  const offer = mgrs.filter((o) => o.via !== "rvm").slice(0, 2); // runa cannot install rvm
  const search = (
    <>
      {needManager && (
        <Callout title={t.tool === "rust" ? "Rust 需要通过 rustup 等版本管理器安装" : `${t.tool === "ruby" ? "Ruby" : t.tool} 没有可直接下载的官方预编译包`}
          action={<span className="flex gap-2">{offer.map((o, i) => (
            <Button key={o.via} size="sm" variant={i === 0 ? "primary" : undefined} disabled={busy} onClick={() => installManagerFor(tool, o.via)}>安装 {providerName(o.via)}</Button>
          ))}</span>}>
          推荐先安装 {offer.map((o) => providerName(o.via)).join(" 或 ")}，之后新版本会通过它安装并自动登记。
        </Callout>
      )}
      <div className="mb-3 flex items-center gap-2">
        <div className="relative flex flex-1 items-center">
          <Search size={16} strokeWidth={1.75} className="pointer-events-none absolute left-2.5 text-fg-3" />
          <Input ref={inputRef} className="w-full pl-8" placeholder="搜索或输入版本，如 20、3.12、latest、lts" value={q}
            onChange={(e) => { setIn("query", tool, e.target.value); setIn("pageSize", tool, 30); }}
            onKeyDown={(e) => { if (e.key === "Enter") install(tool, q.trim() || "latest"); }} />
        </div>
        <Button variant="primary" disabled={busy} onClick={() => install(tool, q.trim() || "latest")}><Download size={14} />安装</Button>
        <Button size="icon" aria-label="刷新版本列表" title="刷新版本列表" onClick={() => loadRemote(tool, true)}>
          <RefreshCw size={14} className={r.status === "loading" ? "animate-spin" : ""} />
        </Button>
      </div>
      <div className="-mt-0.5 mb-3 flex min-h-[18px] flex-wrap items-center gap-1.5 text-sm text-fg-3">
        <InstallerPicker tool={tool} />{" "}
        {q ? <>按 <span className="rounded border border-b-2 border-border bg-surface px-[5px] py-px font-mono text-[11px] text-fg-2">Enter</span> 安装匹配 “{q}” 的最新版本</> : "支持模糊版本：输入主版本号即可安装该系列的最新版"}
      </div>
    </>
  );
  if (r.status === "loading")
    return (
      <>{search}
        <List>{[80, 100, 70, 110, 90, 60].map((w, i) => <Row key={i} className="min-h-12"><div className="flex-1"><Skeleton className="h-3" style={{ width: w }} /></div><Skeleton className="h-6 w-14" /></Row>)}</List>
      </>
    );
  if (r.status === "error")
    return (
      <>{search}
        <Callout kind="danger" title="获取版本列表失败" action={<Button onClick={() => loadRemote(tool, true)}>重试</Button>}>{r.error}</Callout>
        <div className="text-sm text-fg-3">网络受限时可在 <a href="#/settings" className="text-accent-text">设置</a> 中切换国内镜像或配置代理。</div>
      </>
    );
  const have = new Set(t.installed.map((i) => i.version));
  let items = r.items.filter((x) => (f === "lts" ? x.lts : f === "stable" ? !x.prerelease : true));
  if (q) items = items.filter((x) => x.version.startsWith(q.replace(/^v/, "")));
  if (!items.length)
    return <>{search}<List><EmptyState icon={Search} title="没有匹配的版本" desc={q ? `没有以 “${q}” 开头的版本，换个关键词试试。` : "当前筛选条件下没有版本。"} /></List></>;
  return (
    <>{search}
      <List>
        {items.slice(0, n).map((x) => {
          const installed = have.has(x.version) || t.installed.some((iv) => tool === "java" && iv.version.split(".")[0] === x.version);
          return (
            <Row key={x.version} className="min-h-12">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 font-medium">
                  <span className="font-mono">{x.version}</span>
                  {x.lts && <Badge variant="accent">LTS</Badge>}
                  {x.prerelease && <Badge>预览</Badge>}
                </div>
              </div>
              {installed ? <Badge variant="ok"><Check size={12} />已安装</Badge> : (
                <div data-actions="" data-show=""><Button size="sm" variant="ghost" disabled={busy} onClick={() => install(tool, x.version)}><Download size={13} />安装</Button></div>
              )}
            </Row>
          );
        })}
        {items.length > n && (
          <div className="flex justify-center border-t border-border p-3">
            <Button variant="ghost" size="sm" onClick={() => setIn("pageSize", tool, n + 50)}>显示更多（剩余 {items.length - n} 个）</Button>
          </div>
        )}
      </List>
    </>
  );
}
