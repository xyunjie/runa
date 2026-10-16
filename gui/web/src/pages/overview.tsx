import { AlertTriangle, Check, CheckCircle2, ChevronRight, HardDrive, Layers, ScanLine, Terminal } from "lucide-react";
import type * as React from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, List, Row } from "@/components/ui/card";
import { Callout, Mark, Page, PageHead } from "@/components/common";
import { Skeleton } from "@/components/ui/skeleton";
import { useStore } from "@/lib/store";
import { fmtBytes } from "@/lib/utils";
import { setupShell } from "@/lib/actions";
import { SourceChip, usageOf } from "./helpers";

function Stat({ icon: Icon, label, children, small }: { icon: typeof Layers; label: string; children: React.ReactNode; small?: boolean }) {
  return (
    <Card className="px-5 py-4">
      <div className="flex items-center gap-1.5 text-sm text-fg-3"><Icon size={14} strokeWidth={1.75} />{label}</div>
      <div className={small ? "mt-2.5 text-md font-medium" : "mt-1.5 text-[22px] font-semibold tabular-nums tracking-[-0.02em]"}>{children}</div>
    </Card>
  );
}

export function OverviewPage() {
  const ov = useStore((s) => s.ov)!;
  const usage = useStore((s) => s.usage);
  const versions = ov.tools.reduce((a, t) => a + t.installed.length, 0);
  const active = ov.tools.filter((t) => t.current?.version).length;
  const total = usage ? usage.reduce((a, u) => a + u.bytes, 0) : null;
  const act = ov.shells.some((s) => s.configured);
  return (
    <Page>
      <PageHead title="概览" sub="所有运行时的当前版本与环境状态" />
      {!ov.cli_found && (
        <Callout kind="warn" icon={AlertTriangle} title="未找到 runa 命令行程序">Shell 激活与 shims 依赖 CLI。请将 runa 放入 PATH，或与桌面端放在同一目录。</Callout>
      )}
      {!act && (
        <Callout title="自动切换尚未启用" action={<Button variant="primary" disabled={!ov.cli_found} onClick={() => setupShell().catch(() => {})}>为 {ov.default_shell} 启用</Button>}>
          启用后，进入项目目录时会根据 .tool-versions / .nvmrc 等文件自动切换版本。
        </Callout>
      )}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <Stat icon={Layers} label="已安装版本">{versions}</Stat>
        <Stat icon={CheckCircle2} label="已生效工具">{active}<span className="text-[14px] font-normal text-fg-3"> / {ov.tools.length}</span></Stat>
        <Stat icon={HardDrive} label="占用空间">{total == null ? <Skeleton className="inline-block h-[18px] w-[70px]" /> : fmtBytes(total)}</Stat>
        <Stat icon={Terminal} label="自动切换" small>{act ? <Badge variant="ok"><Check size={12} />已启用</Badge> : <Badge variant="warn">未启用</Badge>}</Stat>
      </div>
      <div className="mb-3 mt-8 flex items-center justify-between font-semibold">
        运行时
        <Button asChild variant="ghost" size="sm"><a href="#/detect"><ScanLine size={14} />检测已有安装</a></Button>
      </div>
      <List>
        {ov.tools.map((t) => {
          const v = t.current?.version || (t.current?.system ? "system" : null);
          const missing = !!t.current && !t.current.version && !t.current.system;
          return (
            <Row key={t.tool} onClick={() => (location.hash = `#/tool/${t.tool}`)} className="group cursor-default" role="link" tabIndex={0}
              onKeyDown={(e) => { if (e.key === "Enter") location.hash = `#/tool/${t.tool}`; }}>
              <Mark tool={t.tool} size={32} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 font-medium">{t.tool}{missing && <Badge variant="warn">需要 {t.current!.spec.specs[0]}</Badge>}</div>
                <div className="truncate text-sm text-fg-3">{t.description}</div>
              </div>
              <div className="w-[140px]">{v ? <div className="font-mono">{v}</div> : missing ? <div className="text-warn">未安装</div> : <div className="text-fg-3">未设置</div>}</div>
              <div className="w-[150px]"><SourceChip cur={t.current} /></div>
              <div className="w-[90px] text-right text-sm text-fg-3">{t.installed.length} 个版本<br />{usage ? fmtBytes(usageOf(usage, t.tool)) : ""}</div>
              <ChevronRight size={16} className="text-fg-3 transition-transform group-hover:translate-x-0.5 group-hover:text-fg-2" />
            </Row>
          );
        })}
      </List>
    </Page>
  );
}
