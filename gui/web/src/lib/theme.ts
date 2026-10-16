import { useSyncExternalStore } from "react";

export type ThemePref = "system" | "light" | "dark";
const mql = window.matchMedia("(prefers-color-scheme: dark)");
const subs = new Set<() => void>();

export function themePref(): ThemePref {
  const q = new URLSearchParams(location.search).get("theme");
  return ((q || localStorage.getItem("runa.theme") || "system") as ThemePref);
}
export function isDark() {
  const p = themePref();
  return p === "dark" || (p === "system" && mql.matches);
}
export function applyTheme() {
  document.documentElement.dataset.theme = isDark() ? "dark" : "light";
  subs.forEach((f) => f());
}
export function setTheme(p: ThemePref) {
  localStorage.setItem("runa.theme", p);
  applyTheme();
}
mql.addEventListener("change", applyTheme);
const sub = (f: () => void) => { subs.add(f); return () => subs.delete(f); };
export const useTheme = () => useSyncExternalStore(sub, () => `${themePref()}:${isDark() ? "dark" : "light"}`);
