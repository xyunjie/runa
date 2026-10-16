import { cn } from "@/lib/utils";
export function Skeleton({ className, style }: { className?: string; style?: React.CSSProperties }) {
  return <span className={cn("skel block", className)} style={style} />;
}
