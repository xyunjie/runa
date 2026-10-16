import { useState } from "react";
import { AlertTriangle, Check, CheckCircle2, Link2, RefreshCw } from "lucide-react";
import type { Detected } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { List, Row } from "@/components/ui/card";
import { Callout, EmptyState, Mark, Page, PageHead } from "@/components/common";
import { useStore } from "@/lib/store";
import { adopt, adoptAll, migrate, redetect, useShortPath } from "@/lib/actions";
import { ago, cn } from "@/lib/utils";
import { ProviderBadge, SkeletonRows } from "./helpers";

function AdoptButton({ item }: { item: Detected }) {
  const [busy, setBusy] = useState(false);
  return (
    <Button size="sm" disabled={busy} onClick={() => { setBusy(true); adopt(item).catch(() => {}).finally(() => setBusy(false)); }}>
      <Link2 size={13} />接管
    </Button>
  );
}

export function DetectPage() {
  const d = useStore((s) => s.detected);
  const migration = useStore((s) => s.migration);
  const sp = useShortPath();
  const [adoptingAll, setAdoptingAll] = useState(false);
  let body;
  if (d.status === "idle" || (d.status === "loading" && !d.items.length)) body = <SkeletonRows />;
  else if (d.status === "error") body = <Callout kind="danger" title="检测失败" action={<Button onClick={() => redetect()}>重试</Button>}>{d.error}</Callout>;
  else if (!d.items.length)
    body = <List><EmptyState icon={CheckCircle2} title="没有发现其它安装" desc="PATH、常见目录以及 nvm / pyenv / SDKMAN! / asdf / fnm / Homebrew 中都没有发现 Node、Java、Python 或 Go。" /></List>;
  else
    body = (
      <List>
        {d.items.map((x) => (
          <Row key={`${x.tool}:${x.exe}:${x.version}`} className={cn(x.system_default && "shadow-[inset_3px_0_0_var(--ok)]")}>
            <Mark tool={x.tool} size={32} />
            <div className={cn("min-w-0 flex-1", x.disabled && "opacity-55")}>
              <div className="flex flex-wrap items-center gap-2 font-medium">
                {x.tool} <span className="font-mono">{x.version}</span>
                {x.system_default && <Badge variant="ok" title="系统 PATH 中排在最前，当前终端默认使用这个版本"><Check size={11} />当前全局</Badge>}
                {x.runa_global && <Badge>runa 全局</Badge>}
                <ProviderBadge provider={x.provider} formula={x.formula} />
              </div>
              <div className="selectable truncate font-mono text-sm text-fg-3" title={x.exe}>{sp(x.home)}</div>
            </div>
            <Badge>{x.via}</Badge>
            {x.view_only ? (
              <Badge title="系统自带的运行时直接位于共享目录（如 /usr/bin），只能查看，不能接管或卸载">系统自带 · 仅查看</Badge>
            ) : x.adopted ? (
              <>
                <Badge variant="ok"><Check size={12} />已接管{x.adopted_as && x.adopted_as !== x.version ? ` · ${x.adopted_as}` : ""}</Badge>
                {x.disabled && <Badge>已禁用</Badge>}
              </>
            ) : <AdoptButton item={x} />}
          </Row>
        ))}
      </List>
    );
  return (
    <Page>
      <PageHead title="检测与接管" sub="接管只登记真实安装位置，不创建链接，原有安装不会被移动或修改。">
        <div className="flex items-center gap-3">
          {!!d.createdAt && (
            <span className="flex items-center gap-1.5 text-sm text-fg-3">
              {d.refreshing ? <><RefreshCw size={13} className="animate-spin" />正在检查变化…</> : `上次检测 ${ago(d.createdAt)}`}
            </span>
          )}
          {d.items.some((x) => !x.adopted && !x.view_only) && (
            <Button variant="primary" disabled={d.refreshing || adoptingAll}
              onClick={() => { setAdoptingAll(true); adoptAll().catch(() => {}).finally(() => setAdoptingAll(false)); }}>
              <Link2 size={14} />全部接管
            </Button>
          )}
          <Button disabled={d.refreshing} onClick={() => redetect()}><RefreshCw size={14} className={d.refreshing ? "animate-spin" : ""} />重新检测</Button>
        </div>
      </PageHead>
      {!!migration.length && (
        <Callout kind="warn" icon={AlertTriangle} title={`有 ${migration.length} 个旧版接管记录`} action={<Button onClick={() => migrate().catch(() => {})}>迁移</Button>}>
          旧版本通过符号链接接管，迁移后改为登记真实安装位置：{migration.map((a) => `${a.tool} ${a.version}（${a.action === "convert" ? "转换" : "删除无效记录"}）`).join("、")}
        </Callout>
      )}
      {body}
    </Page>
  );
}
