// Minimal global store (useSyncExternalStore), mirroring the state object of the old UI.
import { useSyncExternalStore } from "react";
import type { Detected, InstallerChoice, ManagerInfo, MigrationAction, Overview, RemoteVersion, Usage } from "./api";

export interface Job {
  stage: string;
  spec?: string;
  version?: string;
  done?: number;
  total?: number | null;
  speed?: number;
  lastT?: number;
  lastDone?: number;
  via?: string;
  command?: string;
  line?: string;
  /** Manager install / upgrade / uninstall job. */
  mgr?: string;
  title?: string;
}

export type Remote = { status: "loading" } | { status: "ok"; items: RemoteVersion[] } | { status: "error"; error: string };

export interface State {
  ov: Overview | null;
  loading: boolean;
  loadError: string | null;
  usage: Usage[] | null;
  project: string;
  remote: Record<string, Remote>;
  tab: Record<string, "installed" | "available">;
  query: Record<string, string>;
  filter: Record<string, "all" | "lts" | "stable">;
  pageSize: Record<string, number>;
  /** tool (or `mgr:<name>`) -> progress */
  jobs: Record<string, Job>;
  installer: Record<string, InstallerChoice>;
  mgrs: ManagerInfo[] | { error: string } | null;
  detected: { status: "idle" | "loading" | "ok" | "error"; items: Detected[]; error: string | null; createdAt: number; refreshing: boolean };
  migration: MigrationAction[];
  projects: string[];
}

let state: State = {
  ov: null, loading: true, loadError: null, usage: null, project: "",
  remote: {}, tab: {}, query: {}, filter: {}, pageSize: {}, jobs: {}, installer: {}, mgrs: null,
  detected: { status: "idle", items: [], error: null, createdAt: 0, refreshing: false },
  migration: [],
  projects: [],
};
const subs = new Set<() => void>();

export const getState = () => state;
export function setState(patch: Partial<State> | ((s: State) => Partial<State>)) {
  state = { ...state, ...(typeof patch === "function" ? patch(state) : patch) };
  subs.forEach((f) => f());
}
/** Update one entry of a record field. `undefined` deletes it. */
export function setIn<K extends "remote" | "tab" | "query" | "filter" | "pageSize" | "jobs" | "installer">(
  key: K, id: string, value: State[K][string] | undefined,
) {
  setState((s) => {
    const next = { ...(s[key] as Record<string, unknown>) };
    if (value === undefined) delete next[id];
    else next[id] = value;
    return { [key]: next } as Partial<State>;
  });
}
const subscribe = (f: () => void) => { subs.add(f); return () => subs.delete(f); };
export function useStore<T>(sel: (s: State) => T): T {
  return useSyncExternalStore(subscribe, () => sel(state));
}
