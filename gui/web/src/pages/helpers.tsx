import { FileText, Globe, Terminal } from "lucide-react";
import type { Resolved, ToolStatus, Usage } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Card, List, Row } from "@/components/ui/card";
import { Page } from "@/components/common";
import { Skeleton } from "@/components/ui/skeleton";
import { providerName } from "@/lib/labels";
import { fileName } from "@/lib/utils";

export function toolStatus(t: ToolStatus, busy: boolean): "" | "ok" | "warn" | "busy" {
  if (busy) return "busy";
  const c = t.current;
  if (c && (c.version || c.system)) return "ok";
  if (c && !c.version) return "warn";
  return "";
}

export function usageOf(usage: Usage[] | null, tool: string, version?: string): number | null {
  if (!usage) return null;
  return usage.filter((u) => u.tool === tool && (version == null || u.version === version)).reduce((a, u) => a + u.bytes, 0);
}

export function SourceChip({ cur }: { cur: Resolved | null }) {
  if (!cur) return <span className="text-fg-3">未设置</span>;
  const s = cur.spec.source;
  const [Icon, label, title] = s.kind === "env" ? [Terminal, s.detail, `环境变量 ${s.detail}`] : s.kind === "global" ? [Globe, "全局", s.detail] : [FileText, fileName(s.detail), s.detail];
  return (
    <span title={title} className="inline-flex h-6 max-w-full items-center gap-1.5 overflow-hidden rounded-md border border-border bg-surface px-2 text-sm text-fg-2">
      <Icon size={13} strokeWidth={1.75} className="flex-none" /><span className="truncate font-mono text-[0.92em]">{label}</span>
    </span>
  );
}

export function ProviderBadge({ provider, formula }: { provider: string | null | undefined; formula?: string | null }) {
  if (!provider) return null;
  return (
    <Badge title={formula ? `${provider}: ${formula}` : provider}>
      {providerName(provider)}{formula ? <> · <span className="font-mono">{formula}</span></> : null}
    </Badge>
  );
}

export function SkeletonRows({ n = 3, mark = true }: { n?: number; mark?: boolean }) {
  return (
    <List>
      {Array.from({ length: n }, (_, i) => (
        <Row key={i}>
          {mark && <Skeleton className="size-8 rounded-lg" />}
          <div className="min-w-0 flex-1"><Skeleton className="h-[11px] w-[120px]" /><Skeleton className="mt-2 h-[9px] w-[220px]" /></div>
        </Row>
      ))}
    </List>
  );
}

export function SkeletonPage() {
  return (
    <Page>
      <Skeleton className="h-6 w-[180px]" />
      <Skeleton className="mb-8 mt-3 h-3 w-[300px]" />
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {[1, 2, 3, 4].map((i) => <Card key={i} className="px-5 py-4"><Skeleton className="h-2.5 w-[60%]" /><Skeleton className="mt-3 h-5 w-[40%]" /></Card>)}
      </div>
      <div className="h-8" />
      <SkeletonRows n={4} />
    </Page>
  );
}
