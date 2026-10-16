import { Toaster as Sonner } from "sonner";
import { useTheme } from "@/lib/theme";

export function Toaster() {
  const t = useTheme();
  return (
    <Sonner
      theme={t.endsWith("dark") ? "dark" : "light"}
      position="bottom-right"
      gap={8}
      toastOptions={{
        classNames: {
          toast: "!rounded-xl !border-border !bg-surface !text-fg !shadow-pop !text-base !items-start",
          title: "!font-medium !text-fg",
          description: "!text-fg-2 !whitespace-pre-wrap !break-words",
          icon: "!mt-0.5",
          success: "[&_[data-icon]]:!text-ok",
          error: "[&_[data-icon]]:!text-danger",
          closeButton: "!bg-surface !border-border !text-fg-3",
        },
      }}
      closeButton
    />
  );
}
