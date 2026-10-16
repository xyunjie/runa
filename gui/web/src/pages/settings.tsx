import { useState } from "react";
import { AlertTriangle, Check, RefreshCw, Terminal } from "lucide-react";
import type * as React from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { List, Row } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { RadioCards } from "@/components/ui/radio-cards";
import { Segmented } from "@/components/ui/segmented";
import { Page, PageHead } from "@/components/common";
import { THEMES } from "@/components/sidebar";
import { useStore } from "@/lib/store";
import { setTheme, themePref, useTheme } from "@/lib/theme";
import { addUserPath, removeShell, reshim, saveProxy, setMirror, setupShell, useShortPath } from "@/lib/actions";

function Group({ title, desc, children }: { title: string; desc: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="grid grid-cols-1 gap-4 border-t border-border py-8 first-of-type:border-t-0 first-of-type:pt-0 xl:grid-cols-[220px_minmax(0,1fr)] xl:gap-8">
      <div><h3 className="m-0 mb-1.5 font-semibold">{title}</h3><p className="m-0 text-sm leading-relaxed text-fg-3">{desc}</p></div>
      <div className="min-w-0">{children}</div>
    </section>
  );
}

type ProxyMode = "system" | "off" | "custom";

export function SettingsPage() {
  const ov = useStore((s) => s.ov)!;
  const sp = useShortPath();
  useTheme();
  const s = ov.settings;
  const savedMode: ProxyMode = s.proxy === "off" || s.proxy === "system" ? s.proxy : "custom";
  const [draft, setDraft] = useState<{ mode: ProxyMode; url: string } | null>(null);
  const [proxyError, setProxyError] = useState<string | null>(null);
  const mode = draft?.mode ?? savedMode;
  const url = draft?.url ?? (mode === "custom" && savedMode === "custom" ? s.proxy : "");
  const save = async (m: ProxyMode, u = "") => {
    const err = await saveProxy(m, u);
    if (err) { setDraft({ mode: m, url: u }); setProxyError(err); } else { setDraft(null); setProxyError(null); }
  };
  return (
    <Page>
      <PageHead title="设置" sub="命令行与桌面端共用同一份配置。" />
      <Group title="下载源" desc="选择运行时的下载来源。国内网络建议使用国内镜像。">
        <RadioCards label="下载源" value={s.mirror} onChange={(m) => setMirror(m).catch(() => {})} options={[
          { value: "off", title: "官方源", desc: "nodejs.org · go.dev · Adoptium · GitHub" },
          { value: "cn", title: "国内镜像", desc: "Node.js 与 Python 使用 npmmirror，Go 使用 golang.google.cn；Java 仍走 Adoptium" },
        ]} />
      </Group>
      <Group title="网络代理" desc="用于访问版本列表与下载。支持 http、https、socks5 与 socks5h。">
        <Segmented<ProxyMode> label="代理模式" className="mb-3" value={mode}
          items={[{ value: "system", label: "跟随系统" }, { value: "off", label: "不使用" }, { value: "custom", label: "自定义" }]}
          onChange={(m) => { if (m === "custom") { setDraft({ mode: m, url: draft?.url || "" }); setProxyError(null); } else save(m); }} />
        {mode === "custom" ? (
          <>
            <div className="flex items-center gap-2">
              <Input autoFocus={!!draft} invalid={!!proxyError} className="flex-1 font-mono text-sm" placeholder="http://127.0.0.1:7890" value={url}
                onChange={(e) => { setDraft({ mode: "custom", url: e.target.value }); setProxyError(null); }}
                onKeyDown={(e) => { if (e.key === "Enter") save("custom", url.trim()); }} />
              <Button variant="primary" onClick={() => save("custom", url.trim())}>保存</Button>
            </div>
            {proxyError && <div className="mt-1.5 flex items-center gap-1.5 text-sm text-danger"><AlertTriangle size={13} />{proxyError}</div>}
          </>
        ) : (
          <p className="text-sm text-fg-3">{mode === "system" ? "读取 HTTPS_PROXY / HTTP_PROXY / ALL_PROXY / NO_PROXY 环境变量。" : "直接连接，不经过任何代理。"}</p>
        )}
      </Group>
      <Group title="Shell 激活" desc="在配置文件中写入带标记的代码块：把 shims 加入 PATH，并在每次提示符时按目录切换版本。修改前会自动备份。重开终端后生效。">
        <List>
          {ov.shells.map((sh) => (
            <Row key={sh.shell}>
              <Terminal size={16} strokeWidth={1.75} className="text-fg-3" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 font-medium">{sh.shell}{sh.shell === ov.default_shell && <Badge>默认</Badge>}</div>
                <div className="selectable truncate font-mono text-sm text-fg-3" title={sh.profile}>{sp(sh.profile)}</div>
              </div>
              {sh.configured && <Badge variant="ok"><Check size={12} />已启用</Badge>}
              <div data-actions="" data-show="">
                {sh.configured ? (
                  <>
                    <Button size="sm" variant="ghost" onClick={() => removeShell(sh.shell).catch(() => {})}>移除</Button>
                    <Button size="sm" onClick={() => setupShell(sh.shell).catch(() => {})}>更新</Button>
                  </>
                ) : <Button size="sm" disabled={!ov.cli_found} onClick={() => setupShell(sh.shell).catch(() => {})}>启用</Button>}
              </div>
            </Row>
          ))}
        </List>
        <div className="mt-3 flex items-center gap-2">
          <Button size="sm" onClick={() => reshim().catch(() => {})}><RefreshCw size={13} />重新生成 shims</Button>
          {ov.is_windows && <Button size="sm" onClick={() => addUserPath().catch(() => {})}>把 shims 加入用户 PATH</Button>}
        </div>
      </Group>
      <Group title="外观" desc="默认跟随系统的浅色 / 深色设置。">
        <Segmented label="主题" value={themePref()} onChange={setTheme}
          items={THEMES.map((t) => ({ value: t.value, label: <><t.icon size={14} strokeWidth={1.75} />{t.label}</> }))} />
      </Group>
      <Group title="关于" desc={`Runa ${ov.app_version}`}>
        <div className="grid grid-cols-[88px_minmax(0,1fr)] gap-x-4 gap-y-2.5 text-sm">
          <span className="text-fg-3">数据目录</span><span className="selectable break-all font-mono text-fg-2">{ov.home}</span>
          <span className="text-fg-3">命令行</span><span className="selectable break-all font-mono text-fg-2">{ov.cli_path}{ov.cli_found ? "" : "（未找到）"}</span>
          <span className="text-fg-3">平台</span><span className="selectable break-all font-mono text-fg-2">{ov.platform}</span>
        </div>
      </Group>
    </Page>
  );
}
