import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva("inline-flex h-5 items-center gap-1 whitespace-nowrap rounded-full px-[7px] text-xs font-medium [&_svg]:shrink-0", {
  variants: {
    variant: {
      default: "bg-hover text-fg-2",
      accent: "bg-accent-soft text-accent-text",
      ok: "bg-ok-soft text-ok",
      warn: "bg-warn-soft text-warn",
    },
  },
  defaultVariants: { variant: "default" },
});

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement>, VariantProps<typeof badgeVariants> {}
export function Badge({ className, variant, ...props }: BadgeProps) {
  return <span className={cn(badgeVariants({ variant }), className)} {...props} />;
}
