import { useEffect, useRef } from "react";
import { AlertTriangle, RefreshCw } from "lucide-react";
import { Sidebar } from "@/components/sidebar";
import { Topbar } from "@/components/topbar";
import { DialogHost } from "@/components/dialogs";
import { Toaster } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { EmptyState, Page } from "@/components/common";
import { useRoute } from "@/lib/route";
import { useStore } from "@/lib/store";
import { events } from "@/lib/api";
import { checkDetect, load, loadDetected, onDetectUpdated, onManagerLine, onProgress } from "@/lib/actions";
import { getState } from "@/lib/store";
import { SkeletonPage } from "@/pages/helpers";
import { OverviewPage } from "@/pages/overview";
import { ToolPage } from "@/pages/tool";
import { DetectPage } from "@/pages/detect";
import { ManagersPage } from "@/pages/managers";
import { SettingsPage } from "@/pages/settings";
import { McpPage } from "@/pages/mcp";
import { ProjectPage, ProjectsPage, loadProjects } from "@/pages/projects";
import { McpApprovalHost } from "@/components/mcp-approval";

function Content() {
  const r = useRoute();
  const loading = useStore((s) => s.loading);
  const err = useStore((s) => s.loadError);
  if (loading) return <SkeletonPage />;
  if (err)
    return (
      <Page>
        <EmptyState icon={AlertTriangle} title="无法读取 runa 数据" desc={err}
          action={<Button variant="primary" onClick={() => load()}><RefreshCw size={14} />重试</Button>} />
      </Page>
    );
  switch (r.name) {
    case "tool": return <ToolPage key={r.arg} name={r.arg || ""} />;
    case "detect": return <DetectPage />;
    case "managers": return <ManagersPage />;
    case "mcp": return <McpPage />;
    case "projects": return <ProjectsPage />;
    case "project": return <ProjectPage key={r.arg} path={r.arg || ""} />;
    case "settings": return <SettingsPage />;
    default: return <OverviewPage />;
  }
}

export function App() {
  const r = useRoute();
  const main = useRef<HTMLDivElement>(null);
  useEffect(() => { main.current?.scrollTo(0, 0); }, [r.name, r.arg]);
  useEffect(() => {
    const un = [events.installProgress(onProgress), events.managerProgress((p) => onManagerLine(p.name, p.line)), events.detectUpdated(onDetectUpdated)];
    const onFocus = () => { if (getState().detected.status !== "idle") checkDetect(); };
    window.addEventListener("focus", onFocus);
    load();
    loadDetected();
    loadProjects();
    return () => { un.forEach((p) => p.then((f) => f())); window.removeEventListener("focus", onFocus); };
  }, []);
  return (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex min-w-0 flex-1 flex-col">
        <Topbar />
        <div ref={main} className="flex-1 overflow-y-auto outline-none" tabIndex={-1}><Content /></div>
      </main>
      <DialogHost />
      <McpApprovalHost />
      <Toaster />
    </div>
  );
}
