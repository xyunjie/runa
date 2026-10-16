import * as React from "react";
import * as D from "@radix-ui/react-dialog";
import { cn } from "@/lib/utils";

export const Dialog = D.Root;
export const DialogTitle = React.forwardRef<HTMLHeadingElement, React.ComponentPropsWithoutRef<typeof D.Title>>(({ className, ...p }, ref) => (
  <D.Title ref={ref} className={cn("m-0 mb-2 text-md font-semibold", className)} {...p} />
));
DialogTitle.displayName = "DialogTitle";
export const DialogDescription = React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<typeof D.Description>>(({ className, ...p }, ref) => (
  <D.Description ref={ref} asChild><div className={cn("leading-relaxed text-fg-2", className)} {...p} /></D.Description>
));
DialogDescription.displayName = "DialogDescription";
export const DialogContent = React.forwardRef<HTMLDivElement, React.ComponentPropsWithoutRef<typeof D.Content> & { wide?: boolean }>(
  ({ className, wide, children, ...p }, ref) => (
    <D.Portal>
      <D.Overlay className="fixed inset-0 z-[60] animate-fade bg-[rgba(10,10,12,.32)] backdrop-blur-[3px]" />
      <div className="pointer-events-none fixed inset-0 z-[61] grid place-items-center">
        <D.Content
          ref={ref}
          className={cn(
            "pointer-events-auto max-h-[calc(100vh-48px)] max-w-[calc(100vw-32px)] animate-pop overflow-y-auto rounded-[14px] border border-border bg-surface p-6 shadow-pop outline-none",
            wide ? "w-[520px]" : "w-[400px]",
            className,
          )}
          {...p}
        >
          {children}
        </D.Content>
      </div>
    </D.Portal>
  ),
);
DialogContent.displayName = "DialogContent";
export function DialogFooter({ children }: { children: React.ReactNode }) {
  return <div className="mt-6 flex justify-end gap-2">{children}</div>;
}
