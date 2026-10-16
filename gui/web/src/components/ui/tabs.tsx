import * as React from "react";
import * as T from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";

export const Tabs = T.Root;
export const TabsContent = T.Content;
export function TabsList({ className, ...p }: React.ComponentPropsWithoutRef<typeof T.List>) {
  return <T.List className={cn("inline-flex gap-0.5 rounded-lg border border-border bg-hover p-0.5", className)} {...p} />;
}
export function TabsTrigger({ className, ...p }: React.ComponentPropsWithoutRef<typeof T.Trigger>) {
  return (
    <T.Trigger
      className={cn(
        "inline-flex h-[26px] items-center gap-1.5 rounded-md px-2.5 text-sm text-fg-2 outline-none transition-[background,color,box-shadow] hover:text-fg focus-visible:outline-2 focus-visible:outline-accent",
        "data-[state=active]:bg-surface data-[state=active]:text-fg data-[state=active]:shadow-[var(--shadow-sm),0_0_0_1px_var(--border)]",
        className,
      )}
      {...p}
    />
  );
}
