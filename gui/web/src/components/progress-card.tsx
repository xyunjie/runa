import { RefreshCw } from "lucide-react";
import { Card } from "./ui/card";
import { useStore } from "@/lib/store";
import { cn, fmtBytes } from "@/lib/utils";

/** The top progress card of a runtime install or a version-manager job (`key` = tool or `mgr:<name>`). */
export function ProgressCard({ id }: { id: string }) {
  const j = useStore((s) => s.jobs[id]);
  if (!j) return null;
  const pct = j.total ? Math.min(100, Math.floor(((j.done || 0) / j.total) * 100)) : null;
  const indet = j.stage !== "downloading" || pct == null;
  const via = j.via || "版本管理器";
  const verb = j.title?.split(" ")[0] || "安装";
  const stageText: Record<string, string> = {
    resolving: "正在解析版本…",
    downloading: "正在下载",
    verifying: "正在校验 SHA256…",
    extracting: "正在解压…",
    manager: `通过 ${via} ${verb}：${j.command || ""}`,
    output: `通过 ${via} ${verb}：${j.line || ""}`,
    done: "完成",
  };
  const title = j.title || `安装 ${j.mgr ? via : `${id} ${j.version || j.spec || ""}`}`;
  return (
    <Card role="status" aria-live="polite" className="mt-6 px-5 py-4">
      <div className="mb-2.5 flex items-baseline justify-between gap-3">
        <div className="flex items-center gap-2 font-medium"><RefreshCw size={14} className="animate-spin" />{title}</div>
        <div className="font-mono text-sm tabular-nums text-fg-2">{pct != null && j.stage === "downloading" ? `${pct}%` : ""}</div>
      </div>
      <div className="h-1 overflow-hidden rounded bg-hover">
        <i className={cn("block h-full rounded bg-accent transition-[width] duration-200 ease-linear", indet && "w-[30%] animate-indet")} style={indet ? undefined : { width: `${pct}%` }} />
      </div>
      <div className="mt-2 flex justify-between gap-4 text-sm tabular-nums text-fg-3">
        <span className="min-w-0 truncate" title={stageText[j.stage]}>{stageText[j.stage] || "准备中…"}</span>
        <span className="flex-none">
          {j.stage === "downloading" ? `${fmtBytes(j.done)}${j.total ? " / " + fmtBytes(j.total) : ""}${j.speed ? " · " + fmtBytes(j.speed) + "/s" : ""}` : ""}
        </span>
      </div>
    </Card>
  );
}
