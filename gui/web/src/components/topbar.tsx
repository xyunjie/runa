import { ChevronRight, Folder, RefreshCw } from "lucide-react";
import { Button } from "./ui/button";
import { useRoute } from "@/lib/route";
import { setState, useStore } from "@/lib/store";
import { load, refreshAll } from "@/lib/actions";

const NAMES: Record<string, string> = { overview: "概览", detect: "检测与接管", managers: "管理器", settings: "设置" };

export function Topbar() {
  const r = useRoute();
  const project = useStore((s) => s.project);
  return (
    <header className="flex h-[52px] flex-none items-center justify-between gap-4 border-b border-border px-6">
      <div className="flex items-center gap-1.5 whitespace-nowrap text-fg-3">
        {r.name === "tool" ? (<><span>运行时</span><ChevronRight size={14} /><b className="font-medium text-fg">{r.arg}</b></>) : <b className="font-medium text-fg">{NAMES[r.name] || "概览"}</b>}
      </div>
      <div className="flex min-w-0 items-center gap-2">
        <label title="项目目录：决定「项目版本」写入哪里，以及当前生效的版本"
          className="flex h-[30px] w-[360px] max-w-[42vw] items-center gap-2 rounded-lg border border-border bg-surface px-3 text-fg-3 transition-[border-color,box-shadow] focus-within:border-accent focus-within:shadow-[0_0_0_3px_var(--accent-soft)]">
          <Folder size={16} strokeWidth={1.75} className="flex-none" />
          <input aria-label="项目目录" spellCheck={false} autoComplete="off" value={project}
            onChange={(e) => setState({ project: e.target.value })}
            onBlur={() => load({ quiet: true })}
            onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur(); }}
            className="w-full border-0 bg-transparent font-mono text-sm text-fg-2 outline-none" />
        </label>
        <Button variant="ghost" size="icon" aria-label="刷新" title="刷新" onClick={() => refreshAll()}><RefreshCw size={16} strokeWidth={1.75} /></Button>
      </div>
    </header>
  );
}
