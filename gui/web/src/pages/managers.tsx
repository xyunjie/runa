import { useEffect } from "react";
import { AlertTriangle, Check, Download, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { List, Row } from "@/components/ui/card";
import { Callout, Page, PageHead } from "@/components/common";
import { ProgressCard } from "@/components/progress-card";
import { setState, useStore } from "@/lib/store";
import { loadManagers, managerAction, managerInstall } from "@/lib/actions";
import { originName, providerName } from "@/lib/labels";
import { SkeletonRows } from "./helpers";

export function ManagersPage() {
  const mgrs = useStore((s) => s.mgrs);
  const jobs = useStore((s) => s.jobs);
  useEffect(() => { if (mgrs == null) loadManagers(); }, [mgrs]);
  let body;
  if (mgrs == null) body = <SkeletonRows mark={false} />;
  else if ("error" in mgrs) body = <Callout kind="danger" icon={AlertTriangle} title="读取失败">{mgrs.error}</Callout>;
  else
    body = (
      <List>
        {mgrs.filter((i) => i.supported).map((i) => {
          const busy = !!jobs[`mgr:${i.name}`];
          const meta = i.installed
            ? [i.version ? `v${i.version}` : "", `安装方式：${originName(i.origin)}`, `管理 ${i.runtimes.length} 个已登记运行时`].filter(Boolean).join(" · ")
            : i.install_method ? `可通过 ${originName(i.install_method.method)} 安装` : "当前没有可用的安装方式";
          const path = i.data_dir || i.entry || "";
          return (
            <Row key={i.name}>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 font-medium">
                  {providerName(i.name)} <span className="font-normal text-fg-3">{i.tools.join(" / ")}</span>
                  {i.installed ? <Badge variant="ok"><Check size={11} />已安装</Badge> : <Badge>未安装</Badge>}
                </div>
                <div className="truncate text-sm text-fg-3">{meta}</div>
                {path && <div className="selectable truncate font-mono text-sm text-fg-3" title={path}>{path}</div>}
              </div>
              <div data-actions="" data-show="">
                {i.installed ? (
                  <>
                    <Button size="sm" disabled={busy} onClick={() => managerAction(i.name, "upgrade")}>升级</Button>
                    <Button size="sm" variant="danger" disabled={busy} onClick={() => managerAction(i.name, "uninstall")}>卸载</Button>
                  </>
                ) : i.install_method ? (
                  <Button size="sm" variant="primary" disabled={busy} onClick={() => managerInstall(i.name)}><Download size={13} />安装</Button>
                ) : null}
              </div>
            </Row>
          );
        })}
      </List>
    );
  return (
    <Page>
      <PageHead title="版本管理器" sub="nvm、fnm、pyenv、SDKMAN!、asdf、mise 等。通过它们安装的运行时会自动登记，卸载也会经过它们。">
        <Button onClick={() => setState({ mgrs: null })}><RefreshCw size={14} />刷新</Button>
      </PageHead>
      {Object.keys(jobs).filter((k) => k.startsWith("mgr:")).map((k) => <div key={k} className="mb-6 [&>*]:mt-0"><ProgressCard id={k} /></div>)}
      {body}
    </Page>
  );
}
