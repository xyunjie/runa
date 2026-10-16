import * as React from "react";
import { cn } from "@/lib/utils";

export function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("rounded-xl border border-border bg-surface shadow-xs", className)} {...props} />;
}
/** Bordered list of rows (the `.list` of the old UI). */
export function List({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("overflow-visible rounded-xl border border-border bg-surface shadow-xs [&>*:first-child]:rounded-t-xl [&>*:last-child]:rounded-b-xl", className)} {...props} />;
}
/** One list row; actions inside `[data-actions]` fade in on hover unless `data-show`. */
export const Row = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(
      "group/row flex min-h-14 flex-wrap items-center gap-x-3.5 gap-y-2 min-w-0 [&>.flex-1]:min-w-[min(220px,100%)] px-4 py-2.5 transition-colors hover:bg-surface-2 [&+&]:border-t [&+&]:border-border",
      "[&_[data-actions]]:ml-auto [&_[data-actions]]:flex [&_[data-actions]]:flex-wrap [&_[data-actions]]:justify-end [&_[data-actions]]:gap-1.5 [&_[data-actions]]:opacity-0 [&_[data-actions]]:transition-opacity hover:[&_[data-actions]]:opacity-100 focus-within:[&_[data-actions]]:opacity-100 [&_[data-actions][data-show]]:opacity-100",
      className,
    )}
    {...props}
  />
));
Row.displayName = "Row";
