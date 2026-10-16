// Promise-based confirm / choice dialogs (shadcn Dialog on Radix), rendered by <DialogHost/>.
import * as React from "react";
import { useSyncExternalStore } from "react";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "./ui/dialog";
import { RadioCards } from "./ui/radio-cards";
import { PlanBox } from "./plan";

interface ConfirmOpts { title: string; body: React.ReactNode; ok?: string; danger?: boolean; cancel?: boolean }
export interface Described { body: React.ReactNode; okDisabled?: boolean; danger?: boolean; okLabel?: string }
interface ChoiceOpts<K extends string> {
  title: string;
  intro?: React.ReactNode;
  options?: { value: K; title: string; desc: string }[];
  initial: K;
  describe: (k: K) => Promise<Described>;
}

type Open =
  | { kind: "confirm"; opts: ConfirmOpts; resolve: (v: boolean) => void }
  | { kind: "choice"; opts: ChoiceOpts<string>; resolve: (v: string | null) => void };

let open: Open | null = null;
const subs = new Set<() => void>();
const set = (o: Open | null) => { open = o; subs.forEach((f) => f()); };
const sub = (f: () => void) => { subs.add(f); return () => subs.delete(f); };

export function confirmDialog(opts: ConfirmOpts): Promise<boolean> {
  return new Promise((resolve) => set({ kind: "confirm", opts, resolve }));
}
export function choiceDialog<K extends string>(opts: ChoiceOpts<K>): Promise<K | null> {
  return new Promise((resolve) => set({ kind: "choice", opts: opts as unknown as ChoiceOpts<string>, resolve: resolve as (v: string | null) => void }));
}

export function DialogHost() {
  const cur = useSyncExternalStore(sub, () => open);
  if (!cur) return null;
  const close = (v: boolean | string | null) => {
    set(null);
    if (cur.kind === "confirm") cur.resolve(v === true);
    else cur.resolve(typeof v === "string" ? v : null);
  };
  return cur.kind === "confirm" ? <Confirm key={Math.random()} opts={cur.opts} close={close} /> : <Choice opts={cur.opts} close={close} />;
}

function Confirm({ opts, close }: { opts: ConfirmOpts; close: (v: boolean) => void }) {
  return (
    <Dialog open onOpenChange={(o) => !o && close(false)}>
      <DialogContent role="alertdialog" onOpenAutoFocus={(e) => { e.preventDefault(); (e.currentTarget as HTMLElement).querySelector<HTMLButtonElement>("[data-cancel]")?.focus(); }} wide>
        <DialogTitle>{opts.title}</DialogTitle>
        <DialogDescription>{opts.body}</DialogDescription>
        <DialogFooter>
          {opts.cancel !== false && <Button data-cancel onClick={() => close(false)}>取消</Button>}
          <Button data-cancel={opts.cancel === false ? true : undefined} variant={opts.danger ? "destructive" : "primary"} onClick={() => close(true)}>{opts.ok || "确定"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Choice({ opts, close }: { opts: ChoiceOpts<string>; close: (v: string | null) => void }) {
  const [cur, setCur] = React.useState(opts.initial);
  const [d, setD] = React.useState<Described | null>(null);
  React.useEffect(() => {
    let live = true;
    setD(null);
    opts.describe(cur)
      .then((x) => live && setD(x))
      .catch((e) => live && setD({ body: <PlanBox danger>{String(e)}</PlanBox>, okDisabled: true }));
    return () => { live = false; };
  }, [cur, opts]);
  return (
    <Dialog open onOpenChange={(o) => !o && close(null)}>
      <DialogContent role="alertdialog" wide onOpenAutoFocus={(e) => { e.preventDefault(); (e.currentTarget as HTMLElement).querySelector<HTMLButtonElement>("[data-cancel]")?.focus(); }}>
        <DialogTitle>{opts.title}</DialogTitle>
        <DialogDescription>{opts.intro}</DialogDescription>
        {!!opts.options?.length && (
          <div className="mt-4"><RadioCards label={opts.title} value={cur} onChange={setCur} options={opts.options} /></div>
        )}
        <div>{d ? d.body : <PlanBox><span className="skel block h-2.5 w-[70%]" /></PlanBox>}</div>
        <DialogFooter>
          <Button data-cancel onClick={() => close(null)}>取消</Button>
          <Button variant={d?.danger ? "destructive" : "primary"} disabled={!d || d.okDisabled} onClick={() => close(cur)}>{d?.okLabel || "确定"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
