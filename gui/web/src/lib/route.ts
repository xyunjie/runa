import { useSyncExternalStore } from "react";

export type RouteName = "overview" | "tool" | "detect" | "managers" | "mcp" | "settings";
export interface Route { name: RouteName; arg?: string }

export function parseRoute(hash = location.hash): Route {
  const [, name = "overview", arg] = (hash || "#/overview").split("/");
  const known: RouteName[] = ["overview", "tool", "detect", "managers", "mcp", "settings"];
  return { name: (known.includes(name as RouteName) ? name : "overview") as RouteName, arg: arg ? decodeURIComponent(arg) : undefined };
}
const sub = (f: () => void) => { window.addEventListener("hashchange", f); return () => window.removeEventListener("hashchange", f); };
export function useRoute(): Route {
  const hash = useSyncExternalStore(sub, () => location.hash);
  return parseRoute(hash);
}
