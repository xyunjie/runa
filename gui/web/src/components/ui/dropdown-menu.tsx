import * as React from "react";
import * as M from "@radix-ui/react-dropdown-menu";
import { cn } from "@/lib/utils";

export const DropdownMenu = M.Root;
export const DropdownMenuTrigger = M.Trigger;
export const DropdownMenuContent = React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<typeof M.Content>>(
  ({ className, sideOffset = 4, ...p }, ref) => (
    <M.Portal>
      <M.Content
        ref={ref}
        sideOffset={sideOffset}
        className={cn("z-[70] min-w-[var(--radix-dropdown-menu-trigger-width)] animate-fade rounded-lg border border-border bg-surface p-1 shadow-pop", className)}
        {...p}
      />
    </M.Portal>
  ),
);
DropdownMenuContent.displayName = "DropdownMenuContent";
export const DropdownMenuItem = React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<typeof M.Item>>(({ className, ...p }, ref) => (
  <M.Item
    ref={ref}
    className={cn("flex cursor-pointer select-none items-center gap-1.5 whitespace-nowrap rounded-md px-2 py-[5px] text-sm outline-none data-[highlighted]:bg-hover", className)}
    {...p}
  />
));
DropdownMenuItem.displayName = "DropdownMenuItem";
