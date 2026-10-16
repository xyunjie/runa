// Card-style radio options (shadcn RadioGroup), like the old `.option` blocks.
import * as RG from "@radix-ui/react-radio-group";
import { cn } from "@/lib/utils";

export function RadioCards<T extends string>({
  value, onChange, options, label,
}: {
  value: T | null;
  onChange: (v: T) => void;
  options: { value: T; title: string; desc: string }[];
  label: string;
}) {
  return (
    <RG.Root value={value ?? undefined} onValueChange={(v) => onChange(v as T)} aria-label={label} className="grid gap-2">
      {options.map((o) => (
        <RG.Item
          key={o.value}
          value={o.value}
          className={cn(
            "group flex items-start gap-3 rounded-xl border border-border bg-surface px-4 py-3 text-left outline-none transition-[border-color,box-shadow] hover:border-border-strong",
            "data-[state=checked]:border-accent data-[state=checked]:shadow-[0_0_0_3px_var(--accent-soft)] focus-visible:border-accent",
          )}
        >
          <span className="mt-0.5 size-4 flex-none rounded-full border-[1.5px] border-border-strong transition-[border] group-data-[state=checked]:border-[5px] group-data-[state=checked]:border-accent" />
          <span>
            <span className="block font-medium text-fg">{o.title}</span>
            <span className="mt-0.5 block text-sm text-fg-3">{o.desc}</span>
          </span>
        </RG.Item>
      ))}
    </RG.Root>
  );
}
