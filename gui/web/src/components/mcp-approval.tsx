// Approval dialog for write operations requested by an AI agent through `runa mcp`.
import { useEffect, useState } from "react";
import { Bot, ShieldAlert } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { api, events, type McpApproval } from "@/lib/api";
import { notify } from "@/lib/actions";

const KIND: Record<string, string> = { install: "安装", set_global: "设为全局版本", uninstall: "卸载" };

export function McpApprovalHost() {
  const [queue, setQueue] = useState<McpApproval[]>([]);
  useEffect(() => {
    const add = (r: McpApproval) => setQueue((q) => (q.some((x) => x.id === r.id) ? q : [...q, r]));
    const un = [
      events.mcpApproval(add),
      events.mcpApprovalDone((id) => setQueue((q) => q.filter((x) => x.id !== id))),
    ];
    api.mcpPending().then((l) => (l || []).forEach(add)).catch(() => {});
    return () => { un.forEach((p) => p.then((f) => f())); };
  }, []);
  const cur = queue[0];
  const answer = async (approved: boolean) => {
    if (!cur) return;
    setQueue((q) => q.slice(1));
    try { await api.mcpRespond(cur.id, approved); }
    catch (e) { notify("确认未送达", String(e), "error"); }
  };
  const danger = cur?.op.kind === "uninstall";
  return (
    <Dialog open={!!cur} onOpenChange={(o) => { if (!o) answer(false); }}>
      {cur && (
        <DialogContent wide aria-describedby={undefined}>
          <DialogTitle className="flex items-center gap-2">
            {danger ? <ShieldAlert size={18} className="text-danger" /> : <Bot size={18} className="text-accent-text" />}
            AI 请求{KIND[cur.op.kind] || cur.op.kind}
          </DialogTitle>
          <DialogDescription>
            <p className="mb-3 mt-0">
              {cur.client ? <><Badge className="mr-1">{cur.client}</Badge>通过 MCP 请求执行以下操作。</> : "一个 AI 客户端通过 MCP 请求执行以下操作。"}
              批准只对这一次操作有效。
            </p>
            <div className="mb-3 rounded-lg border border-border bg-surface-2 px-3 py-2.5">
              <div className="font-medium text-fg">{cur.summary}</div>
              <div className="mt-1 font-mono text-xs text-fg-3">
                {cur.op.tool} {cur.op.version}{cur.op.via ? ` · ${cur.op.via}` : ""}{cur.op.mode ? ` · ${cur.op.mode}` : ""}
              </div>
            </div>
            {cur.details.length > 0 && (
              <ul className="selectable m-0 max-h-40 list-none overflow-y-auto p-0 font-mono text-xs text-fg-2">
                {cur.details.map((d, i) => <li key={i} className="break-all py-0.5">{d}</li>)}
              </ul>
            )}
            {queue.length > 1 && <p className="mb-0 mt-3 text-xs text-fg-3">还有 {queue.length - 1} 个请求等待确认</p>}
          </DialogDescription>
          <DialogFooter>
            <Button onClick={() => answer(false)}>拒绝</Button>
            <Button variant={danger ? "destructive" : "primary"} onClick={() => answer(true)}>批准{KIND[cur.op.kind] || ""}</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  );
}
