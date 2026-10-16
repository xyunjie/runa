import * as React from "react";
import { cn } from "@/lib/utils";

export const Input = React.forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement> & { invalid?: boolean }>(
  ({ className, invalid, ...props }, ref) => (
    <input
      ref={ref}
      spellCheck={false}
      autoComplete="off"
      className={cn(
        "h-8 min-w-0 rounded-lg border border-border bg-surface px-3 text-base text-fg outline-none transition-[border-color,box-shadow] placeholder:text-fg-3 focus:border-accent focus:shadow-[0_0_0_3px_var(--accent-soft)]",
        invalid && "border-danger shadow-[0_0_0_3px_var(--danger-soft)]",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";
