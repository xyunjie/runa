import { Logo } from "@/components/logo";
import { Bot, FolderGit2, FolderPlus, LayoutGrid, Monitor, Moon, Package, ScanLine, Settings, Sun } from "lucide-react";
import type * as React from "react";
import { Dot, Mark } from "./common";
import { Segmented } from "./ui/segmented";
import { Skeleton } from "./ui/skeleton";
import { useStore } from "@/lib/store";
import { useRoute } from "@/lib/route";
import { setTheme, themePref, useTheme, type ThemePref } from "@/lib/theme";
import { toolStatus } from "@/pages/helpers";
import { cn } from "@/lib/utils";

function NavItem({ href, active, children }: { href: string; active: boolean; children: React.ReactNode }) {
  return (
    <a href={href} aria-current={active ? "page" : undefined}
      className={cn("flex h-8 select-none items-center gap-2.5 rounded-md px-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg", active && "bg-active font-medium text-fg")}>
      {children}
    </a>
  );
}
const Label = ({ children }: { children: React.ReactNode }) => <div className="px-2 pb-2 pt-5 text-xs font-medium text-fg-3">{children}</div>;

export const THEMES: { value: ThemePref; icon: typeof Sun; label: string }[] = [
  { value: "system", icon: Monitor, label: "跟随系统" },
  { value: "light", icon: Sun, label: "浅色" },
  { value: "dark", icon: Moon, label: "深色" },
];

export function Sidebar() {
  const r = useRoute();
  const ov = useStore((s) => s.ov);
  const jobs = useStore((s) => s.jobs);
  const projects = useStore((s) => s.projects);
  useTheme();
  return (
    <aside className="flex w-[232px] flex-none flex-col border-r border-border bg-sidebar">
      <div className="flex items-center gap-2.5 px-5 pb-4 pt-5">
        <Logo size={22} />
        <span className="text-md font-semibold tracking-[-0.01em]">Runa</span>
      </div>
      <nav aria-label="主导航" className="flex-1 overflow-y-auto px-3">
        <NavItem href="#/overview" active={r.name === "overview"}><LayoutGrid size={16} strokeWidth={1.75} />概览</NavItem>
        <Label>运行时</Label>
        {!ov
          ? [1, 2, 3, 4].map((i) => <div key={i} className="flex h-8 items-center gap-2.5 px-2"><Skeleton className="size-5" /><Skeleton className="h-2.5 flex-1" /></div>)
          : ov.tools.map((t) => (
              <NavItem key={t.tool} href={`#/tool/${t.tool}`} active={r.name === "tool" && r.arg === t.tool}>
                <Mark tool={t.tool} size={20} />
                <span className="flex-1">{t.tool}</span>
                <span className="font-mono text-xs text-fg-3">{t.current?.version || (t.current?.system ? "system" : "")}</span>
                <Dot status={toolStatus(t, !!jobs[t.tool])} />
              </NavItem>
            ))}
        <Label>项目</Label>
        {projects.map((p) => (
          <NavItem key={p} href={`#/project/${encodeURIComponent(p)}`} active={r.name === "project" && r.arg === p}>
            <FolderGit2 size={16} strokeWidth={1.75} className="flex-none" />
            <span className="min-w-0 flex-1 truncate" title={p}>{p.split(/[\\/]/).filter(Boolean).pop() || p}</span>
          </NavItem>
        ))}
        <NavItem href="#/projects" active={r.name === "projects"}><FolderPlus size={16} strokeWidth={1.75} />{projects.length ? "管理项目" : "添加项目"}</NavItem>
        <Label>管理</Label>
        <NavItem href="#/detect" active={r.name === "detect"}><ScanLine size={16} strokeWidth={1.75} />检测与接管</NavItem>
        <NavItem href="#/managers" active={r.name === "managers"}><Package size={16} strokeWidth={1.75} />管理器</NavItem>
        <NavItem href="#/mcp" active={r.name === "mcp"}><Bot size={16} strokeWidth={1.75} />MCP</NavItem>
        <NavItem href="#/settings" active={r.name === "settings"}><Settings size={16} strokeWidth={1.75} />设置</NavItem>
      </nav>
      <div className="px-4 pb-5 pt-4">
        <Segmented small label="主题" value={themePref()} onChange={setTheme}
          items={THEMES.map((t) => ({ value: t.value, title: t.label, label: <t.icon size={14} strokeWidth={1.75} /> }))} />
      </div>
    </aside>
  );
}
