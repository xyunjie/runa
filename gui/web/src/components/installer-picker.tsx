// 安装方式 dropdown: concrete choices only (managers in preference order, then runa 下载);
// the recommended one is tagged 推荐 and selecting it stores `auto`; uninstalled managers are
// greyed and open the install confirmation instead of being selected.
import { ChevronDown } from "lucide-react";
import { Badge } from "./ui/badge";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { useStore } from "@/lib/store";
import { viaLabel } from "@/lib/labels";
import { installManagerFor, setInstaller } from "@/lib/actions";
import { cn } from "@/lib/utils";

const Tag = () => <Badge className="h-[18px] px-[5px] text-[10.5px]">推荐</Badge>;

export function InstallerPicker({ tool }: { tool: string }) {
  const c = useStore((s) => s.installer[tool]);
  const busy = useStore((s) => !!s.jobs[tool]);
  if (!c) return null;
  const managers = c.options.filter((o) => o.via !== "runa");
  const items = [...managers, { via: "runa", available: true, detail: "" }].filter((o, i, a) => a.findIndex((x) => x.via === o.via) === i);
  const rec = managers.find((o) => o.available)?.via || "runa";
  const sel = c.preference === "auto" || !items.some((o) => o.via === c.preference && o.available) ? rec : c.preference;
  return (
    <span className="inline-flex items-center gap-2">
      安装方式
      <DropdownMenu>
        <DropdownMenuTrigger
          disabled={busy}
          title="新版本的安装方式；安装后自动登记，卸载也会通过它"
          className="inline-flex h-7 min-w-[140px] items-center gap-1.5 rounded-lg border border-border bg-surface px-2 text-left text-sm text-fg outline-none transition-[border-color,box-shadow] focus-visible:border-accent focus-visible:shadow-[0_0_0_3px_var(--accent-soft)] disabled:opacity-45"
        >
          {viaLabel(sel)}
          {sel === rec && <Tag />}
          <ChevronDown size={12} className="ml-auto text-fg-3" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" aria-label="安装方式" loop>
          {items.map((o) =>
            o.available ? (
              <DropdownMenuItem key={o.via} aria-checked={o.via === sel} role="menuitemradio" className={cn(o.via === sel && "font-semibold")}
                onSelect={() => setInstaller(tool, o.via === rec ? "auto" : o.via)}>
                {viaLabel(o.via)}{o.via === rec && <Tag />}
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem key={o.via} aria-disabled="true" title="未安装，点击安装" className="text-fg-3 opacity-60"
                onSelect={() => installManagerFor(tool, o.via)}>
                {viaLabel(o.via)}
              </DropdownMenuItem>
            ),
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    </span>
  );
}
