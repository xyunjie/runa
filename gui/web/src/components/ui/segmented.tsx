// Segmented control (shadcn ToggleGroup, single selection, styled like the old `.seg`).
import * as React from "react";
import * as TG from "@radix-ui/react-toggle-group";
import { cn } from "@/lib/utils";

export function Segmented<T extends string>({
  value, onChange, items, label, className, small,
}: {
  value: T;
  onChange: (v: T) => void;
  items: { value: T; label: React.ReactNode; title?: string }[];
  label: string;
  className?: string;
  small?: boolean;
}) {
  return (
    <TG.Root
      type="single"
      value={value}
      onValueChange={(v) => v && onChange(v as T)}
      aria-label={label}
      className={cn("inline-flex gap-0.5 rounded-lg border border-border bg-hover p-0.5", small && "flex w-full", className)}
    >
      {items.map((it) => (
        <TG.Item
          key={it.value}
          value={it.value}
          title={it.title}
          aria-label={it.title}
          className={cn(
            "inline-flex h-[26px] items-center gap-1.5 rounded-md px-2.5 text-sm text-fg-2 outline-none transition-[background,color,box-shadow] hover:text-fg focus-visible:outline-2 focus-visible:outline-accent",
            "data-[state=on]:bg-surface data-[state=on]:text-fg data-[state=on]:shadow-[var(--shadow-sm),0_0_0_1px_var(--border)]",
            small && "h-6 flex-1 justify-center p-0",
          )}
        >
          {it.label}
        </TG.Item>
      ))}
    </TG.Root>
  );
}
