import { useEffect, useState } from "react";
import { AlertTriangle, Check, ChevronDown, ChevronUp, Copy, Link2, Plus, RefreshCw, Trash2 } from "lucide-react";
import type * as React from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { List, Row } from "@/components/ui/card";
import { RadioCards } from "@/components/ui/radio-cards";
import { Callout, Page, PageHead } from "@/components/common";
import { api, type McpClient, type McpPolicy, type McpStatus } from "@/lib/api";
import { notify, useShortPath } from "@/lib/actions";

function Group({ title, desc, children }: { title: string; desc: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="grid grid-cols-1 gap-4 border-t border-border py-8 first-of-type:border-t-0 first-of-type:pt-0 lg:grid-cols-[240px_1fr] lg:gap-8">
      <div><h3 className="m-0 mb-1.5 font-semibold">{title}</h3><div className="m-0 text-sm leading-relaxed text-fg-3">{desc}</div></div>
      <div>{children}</div>
    </section>
  );
}

function Toggle({ checked, onChange, title, desc, disabled }: { checked: boolean; onChange: (v: boolean) => void; title: string; desc: string; disabled?: boolean }) {
  return (
    <label className={"flex cursor-pointer items-start gap-3 py-2" + (disabled ? " opacity-50" : "")}>
      <input type="checkbox" className="mt-0.5 size-4 accent-[var(--accent)]" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span><span className="block font-medium">{title}</span><span className="block text-sm text-fg-3">{desc}</span></span>
    </label>
  );
}

async function copy(text: string) {
  try { await navigator.clipboard.writeText(text); notify("已复制配置"); }
  catch (e) { notify("复制失败", String(e), "error"); }
}

function ClientRow({ c, cliFound, busy, onAdd, onRemove }: { c: McpClient; cliFound: boolean; busy: boolean; onAdd: () => void; onRemove: () => void }) {
  const [open, setOpen] = useState(false);
  const sp = useShortPath();
  const how = c.method === "deeplink" ? "通过安装链接添加，需在客户端中确认" : c.method === "cli" ? (c.cli ? "通过 claude mcp add 添加（用户级）" : "未找到 claude 命令，请复制命令手动执行") : "直接写入配置文件（修改前自动备份）";
  const canAdd = cliFound && !(c.method === "cli" && !c.cli);
  const canRemove = c.added && !(c.method === "cli" && !c.cli);
  return (
    <div className="[&+&]:border-t [&+&]:border-border">
      <Row className="border-0">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 font-medium">
            {c.name}
            {c.added && <Badge variant="ok"><Check size={12} />已添加</Badge>}
          </div>
          <div className="truncate text-sm text-fg-3">{how}</div>
          {c.config_path && <div className="selectable truncate font-mono text-xs text-fg-3" title={c.config_path}>{sp(c.config_path)}</div>}
        </div>
        <div data-actions="" data-show="">
          <Button size="sm" variant="ghost" onClick={() => setOpen(!open)}>{open ? <ChevronUp size={13} /> : <ChevronDown size={13} />}复制配置</Button>
          {canRemove && <Button size="sm" variant="danger" disabled={busy} onClick={onRemove}><Trash2 size={13} />移除</Button>}
          <Button size="sm" variant={c.added ? "default" : "primary"} disabled={busy || !canAdd} onClick={onAdd}>
            {c.method === "deeplink" ? <Link2 size={13} /> : <Plus size={13} />}{c.added ? "重新添加" : "一键添加"}
          </Button>
        </div>
      </Row>
      {open && (
        <div className="px-4 pb-4">
          <div className="relative">
            <pre className="selectable m-0 max-h-64 overflow-auto rounded-lg border border-border bg-surface-2 p-3 font-mono text-xs leading-relaxed text-fg-2">{c.snippet}</pre>
            <Button size="sm" className="absolute right-2 top-2" onClick={() => copy(c.snippet)}><Copy size={13} />复制</Button>
          </div>
          {c.config_path && <p className="mb-0 mt-2 text-xs text-fg-3">把 runa 项合并到 {sp(c.config_path)}（保留已有内容），然后重启 {c.name}。</p>}
        </div>
      )}
    </div>
  );
}

export function McpPage() {
  const [st, setSt] = useState<McpStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const refresh = () => api.mcpStatus().then((s) => { setSt(s); setErr(null); }).catch((e) => setErr(String(e)));
  useEffect(() => {
    refresh();
    const f = () => refresh();
    window.addEventListener("focus", f);
    return () => window.removeEventListener("focus", f);
  }, []);
  const sp = useShortPath();
  if (err) return <Page><PageHead title="MCP" /><Callout kind="danger" title="无法读取 MCP 状态">{err}</Callout></Page>;
  if (!st) return <Page><PageHead title="MCP" sub="加载中…" /></Page>;
  const p = st.policy;
  const save = async (np: McpPolicy) => {
    try { const r = await api.mcpSetPolicy(np); setSt({ ...st, policy: r }); notify("已保存 MCP 设置"); }
    catch (e) { notify("保存失败", String(e), "error"); }
  };
  const act = async (c: McpClient, remove: boolean) => {
    setBusy(c.id);
    try { notify(await (remove ? api.mcpRemove(c.id) : api.mcpAdd(c.id))); }
    catch (e) { notify(remove ? "移除失败" : "添加失败", String(e), "error"); }
    setBusy(null);
    refresh();
  };
  return (
    <Page>
      <PageHead title="MCP" sub="让 Cursor、Claude 等 AI 客户端通过 MCP 查询和管理本机的运行时。">
        <Button size="sm" onClick={refresh}><RefreshCw size={13} />刷新</Button>
      </PageHead>
      {!st.cli_found && <Callout kind="warn" title="未找到 runa 命令行程序">AI 客户端需要通过 runa 命令启动 MCP 服务。请把 runa 放到 PATH 中或与 Runa 同目录。</Callout>}
      {st.bridge_error && <Callout kind="danger" icon={AlertTriangle} title="确认服务未启动">{st.bridge_error}；需要确认的操作会被拒绝。</Callout>}
      <Group title="状态" desc="AI 客户端以 stdio 方式启动下面这个命令。">
        <div className="grid grid-cols-[96px_1fr] gap-x-4 gap-y-2.5 text-sm">
          <span className="text-fg-3">命令</span><span className="selectable break-all font-mono text-fg-2">{sp(st.cli_path)} mcp</span>
          <span className="text-fg-3">确认服务</span><span className="text-fg-2">{st.bridge_running ? "运行中（仅本机 127.0.0.1）" : "未运行"}</span>
          <span className="text-fg-3">工具</span><span className="text-fg-2">list_runtimes、list_available、detect_project、list_managers（只读）；install、set_global、uninstall（写入）</span>
        </div>
      </Group>
      <Group title="权限" desc="对所有 AI 客户端生效，每次调用时重新读取。">
        <RadioCards label="MCP 权限" value={p.mode} onChange={(m) => save({ ...p, mode: m as McpPolicy["mode"] })} options={[
          { value: "read_only", title: "只读", desc: "只能查询已安装版本、可安装版本、项目要求和管理器，拒绝任何修改" },
          { value: "allow_write", title: "允许写入", desc: "允许安装、切换全局版本和卸载；系统自带的运行时始终不可卸载" },
        ]} />
        <div className="mt-3">
          <Toggle checked={p.confirm_destructive} disabled={p.mode !== "allow_write"} onChange={(v) => save({ ...p, confirm_destructive: v })}
            title="卸载前需要在 Runa 中确认" desc="AI 请求卸载时弹出确认对话框；Runa 未打开时直接拒绝。" />
          <Toggle checked={p.confirm_writes} disabled={p.mode !== "allow_write"} onChange={(v) => save({ ...p, confirm_writes: v })}
            title="安装和切换全局版本也需要确认" desc="开启后，所有写入操作都需要在 Runa 中逐一批准。" />
        </div>
      </Group>
      <Group title="AI 客户端" desc={<>“一键添加”使用客户端的安装链接、命令或直接写入其用户级配置；“复制配置”可手动添加。修改配置后请重启客户端。</>}>
        <List>
          {st.clients.map((c) => (
            <ClientRow key={c.id} c={c} cliFound={st.cli_found} busy={busy === c.id} onAdd={() => act(c, false)} onRemove={() => act(c, true)} />
          ))}
        </List>
        <p className="mt-2 text-xs text-fg-3">Cursor、VS Code 通过安装链接添加，“已添加”根据其用户级配置文件判断，确认安装后点“刷新”。</p>
      </Group>
    </Page>
  );
}
