import type * as React from "react";
import { AlertTriangle, Info, Zap, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";

const MARKS: Record<string, { label: string; color: string }> = {
  node: { label: "N", color: "#5FA04E" },
  java: { label: "J", color: "#E76F00" },
  python: { label: "Py", color: "#3776AB" },
  go: { label: "Go", color: "#00ADD8" },
  rust: { label: "Rs", color: "#CE422B" },
  ruby: { label: "Rb", color: "#CC342D" },
  bun: { label: "B", color: "#F9A8D4" },
  deno: { label: "D", color: "#3FB37F" },
};

/** Tinted monogram badge in the language's brand color. */
export function Mark({ tool, size = 24 }: { tool: string; size?: number }) {
  const m = MARKS[tool] || { label: (tool[0] || "?").toUpperCase(), color: "#8E8E96" };
  const fs = Math.round(size * (m.label.length > 1 ? 0.4 : 0.5));
  return (
    <span
      aria-hidden
      className="mark inline-grid flex-none place-items-center rounded-[28%] font-[650] tracking-[-0.02em]"
      style={{ "--mc": m.color, width: size, height: size, fontSize: fs } as React.CSSProperties}
    >
      {m.label}
    </span>
  );
}

export function EmptyState({ icon: Icon, title, desc, action }: { icon: LucideIcon; title: string; desc: string; action?: React.ReactNode }) {
  return (
    <div className="px-6 py-12 text-center">
      <div className="mx-auto mb-4 grid size-11 place-items-center rounded-xl bg-hover text-fg-3"><Icon size={20} strokeWidth={1.75} /></div>
      <div className="text-md font-medium">{title}</div>
      <div className="mx-auto mb-5 mt-1.5 max-w-[380px] text-fg-3">{desc}</div>
      {action}
    </div>
  );
}

const CALLOUT = {
  info: { cls: "bg-accent-soft border-[color-mix(in_srgb,var(--accent)_20%,transparent)]", ic: "text-accent-text", Icon: Zap },
  warn: { cls: "bg-warn-soft border-[color-mix(in_srgb,var(--warn)_25%,transparent)]", ic: "text-warn", Icon: AlertTriangle },
  danger: { cls: "bg-danger-soft border-[color-mix(in_srgb,var(--danger)_25%,transparent)]", ic: "text-danger", Icon: AlertTriangle },
  note: { cls: "bg-accent-soft border-[color-mix(in_srgb,var(--accent)_20%,transparent)]", ic: "text-accent-text", Icon: Info },
};
export function Callout({ kind = "info", title, children, action, className, icon }: {
  kind?: keyof typeof CALLOUT; title: React.ReactNode; children?: React.ReactNode; action?: React.ReactNode; className?: string; icon?: LucideIcon;
}) {
  const c = CALLOUT[kind];
  const Icon = icon || c.Icon;
  return (
    <div className={cn("mb-6 flex items-center gap-3 rounded-xl border px-4 py-3 text-fg", c.cls, className)}>
      <Icon size={16} strokeWidth={1.75} className={cn("flex-none", c.ic)} />
      <div className="flex-1">
        <div className="font-medium">{title}</div>
        {children && <div className="text-sm text-fg-2">{children}</div>}
      </div>
      {action}
    </div>
  );
}

export function PageHead({ title, sub, children }: { title: React.ReactNode; sub?: React.ReactNode; children?: React.ReactNode }) {
  return (
    <div className="mb-8 flex items-start justify-between gap-6">
      <div>
        <h1 className="m-0 text-xl font-semibold leading-tight tracking-[-0.02em]">{title}</h1>
        {sub && <div className="mt-1.5 text-fg-3">{sub}</div>}
      </div>
      {children}
    </div>
  );
}

export function Page({ children }: { children: React.ReactNode }) {
  return <div className="mx-auto max-w-[960px] animate-enter px-8 pb-12 pt-10">{children}</div>;
}

export const Mono = ({ children, className, title }: { children: React.ReactNode; className?: string; title?: string }) => (
  <span className={cn("font-mono text-[0.92em]", className)} title={title}>{children}</span>
);

export function Dot({ status }: { status: "" | "ok" | "warn" | "busy" }) {
  return (
    <span
      className={cn(
        "size-1.5 flex-none rounded-full bg-border-strong",
        status === "ok" && "bg-ok",
        status === "warn" && "bg-warn",
        status === "busy" && "animate-pulse-dot bg-accent",
      )}
    />
  );
}
