import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-lg border font-medium transition-[background,border-color,color,box-shadow,transform] duration-150 active:translate-y-px disabled:pointer-events-none disabled:opacity-45 [&_svg]:shrink-0 [&_svg]:pointer-events-none outline-none focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2",
  {
    variants: {
      variant: {
        default: "border-border bg-surface text-fg shadow-xs hover:bg-surface-2 hover:border-border-strong",
        primary: "border-transparent bg-accent text-on-accent shadow-xs hover:bg-accent-hover",
        ghost: "border-transparent bg-transparent text-fg-2 shadow-none hover:bg-hover hover:text-fg",
        danger: "border-border bg-surface text-danger shadow-xs hover:bg-danger-soft hover:border-[color-mix(in_srgb,var(--danger)_30%,transparent)]",
        destructive: "border-transparent bg-danger text-white shadow-xs hover:bg-[color-mix(in_srgb,var(--danger)_88%,#000)]",
      },
      size: {
        default: "h-[30px] px-3 text-sm",
        sm: "h-[26px] px-2.5 text-xs rounded-md",
        icon: "size-[30px] p-0",
        "icon-sm": "size-[26px] p-0 rounded-md",
      },
    },
    defaultVariants: { variant: "default", size: "default" },
  },
);

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(({ className, variant, size, asChild, type, ...props }, ref) => {
  const Comp = asChild ? Slot : "button";
  return <Comp ref={ref} type={asChild ? undefined : (type ?? "button")} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
});
Button.displayName = "Button";
export { buttonVariants };
