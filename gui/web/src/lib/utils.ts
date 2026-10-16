import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function fmtBytes(n: number | null | undefined): string {
  if (n == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(n < 10 && i ? 1 : 0)} ${u[i]}`;
}

export const fileName = (p: string | null | undefined) => String(p || "").split(/[\\/]/).pop() || "";

export function ago(secs: number): string {
  if (!secs) return "";
  const d = Math.max(0, Date.now() / 1000 - secs);
  return d < 60 ? "刚刚" : d < 3600 ? `${Math.floor(d / 60)} 分钟前` : d < 86400 ? `${Math.floor(d / 3600)} 小时前` : `${Math.floor(d / 86400)} 天前`;
}
