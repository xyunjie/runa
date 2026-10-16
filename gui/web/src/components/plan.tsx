import type * as React from "react";
import type { ManagerPlan, UninstallPlan } from "@/lib/api";
import { cn } from "@/lib/utils";
import { useShortPath } from "@/lib/actions";

export function PlanBox({ children, danger }: { children: React.ReactNode; danger?: boolean }) {
  return (
    <div className={cn("selectable mt-3 max-h-[260px] overflow-auto rounded-[10px] border border-border bg-surface-2 p-3 text-sm text-fg-2", danger && "border-[color-mix(in_srgb,var(--danger)_35%,transparent)]")}>
      {children}
    </div>
  );
}
const Lbl = ({ children }: { children: React.ReactNode }) => <div className="mt-2 text-fg-3 first:mt-0">{children}</div>;
const Code = ({ children }: { children: React.ReactNode }) => <span className="mt-1 block break-all font-mono text-[0.92em] text-fg">{children}</span>;

/** Runtime uninstall plan (forget / purge / delete). */
export function UninstallPlanView({ p }: { p: UninstallPlan }) {
  const sp = useShortPath();
  return (
    <PlanBox danger={p.kind !== "forget"}>
      <Lbl>{p.summary}</Lbl>
      {p.command_line && (
        <>
          <Lbl>{p.needs_root ? "需要管理员权限，runa 不会自动 sudo，请在终端手动执行：" : "将执行命令："}</Lbl>
          <Code>{p.command_line}</Code>
        </>
      )}
      {!!p.paths.length && (
        <>
          <Lbl>{p.kind === "forget" ? "将删除的 runa 记录：" : "将删除："}</Lbl>
          {p.paths.map((x) => <Code key={x}>{sp(x)}</Code>)}
        </>
      )}
      {p.kind === "forget" && p.home && (
        <>
          <Lbl>保留原有安装：</Lbl>
          <Code>{sp(p.home)}</Code>
        </>
      )}
    </PlanBox>
  );
}

/** Version-manager upgrade / uninstall plan. */
export function ManagerPlanView({ p }: { p: ManagerPlan }) {
  const uninstall = p.action === "uninstall";
  return (
    <PlanBox danger={uninstall && !p.keep_data}>
      <Lbl>{p.summary}</Lbl>
      {p.command_line && (<><Lbl>将执行：</Lbl><Code>{p.command_line}</Code></>)}
      {!!p.remove_dirs.length && (<><Lbl>将删除：</Lbl>{p.remove_dirs.map((d) => <Code key={d}>{d}</Code>)}</>)}
      {!!p.rc_edits.length && (
        <>
          <Lbl>将从 shell 配置文件移除以下行（原文件备份为 .runa-bak）：</Lbl>
          {p.rc_edits.map((e) => (
            <div key={e.file}>
              <Code>{e.file}</Code>
              {e.lines.map(([n, l]) => <Code key={n}><span className="text-fg-3">{n}: </span>{l}</Code>)}
            </div>
          ))}
        </>
      )}
      {uninstall && p.keep_data && !!p.data_runtimes.length && (
        <>
          <Lbl>保留已安装的版本：</Lbl>
          {p.data_runtimes.map((d) => <Code key={d}>{d}</Code>)}
          {!!p.runtimes.length && <Lbl>{p.runtimes.map((r) => `${r.tool} ${r.version}`).join("、")} 保持登记，来源改为“手动安装”</Lbl>}
        </>
      )}
      {uninstall && !p.keep_data && !!p.data_runtimes.length && (
        <><Lbl>以下已安装的版本会一起删除：</Lbl>{p.data_runtimes.map((d) => <Code key={d}>{d}</Code>)}</>
      )}
      {uninstall && !p.keep_data && !!p.runtimes.length && (
        <div className="mt-3 rounded-lg border border-[color-mix(in_srgb,var(--warn)_25%,transparent)] bg-warn-soft px-3 py-2 text-fg">
          <div className="font-medium">以下运行时也会被删除并从 runa 取消登记</div>
          <div className="text-fg-2">{p.runtimes.map((r) => `${r.tool} ${r.version}${r.global ? "（runa 全局）" : ""}`).join("、")}</div>
        </div>
      )}
    </PlanBox>
  );
}
