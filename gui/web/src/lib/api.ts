// Typed wrappers around the Tauri IPC commands and events of gui/src-tauri/src/main.rs.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { mockInvoke, mockListen } from "./mock";

// ---------------------------------------------------------------- types
export interface Source { kind: "env" | "global" | "file"; detail: string }
export interface Spec { tool: string; specs: string[]; source: Source }
export interface Resolved { tool: string; spec: Spec; version: string | null; system: boolean }
export interface InstalledVersion {
  version: string;
  source: string; // "download" | "adopted" | ...
  path: string;
  origin: string | null;
  provider: string | null;
  formula: string | null;
  disabled: boolean;
}
export interface ToolStatus {
  tool: string;
  description: string;
  builtin: boolean;
  installed: InstalledVersion[];
  global: string | null;
  current: Resolved | null;
}
export interface MirrorRule { from: string; to: string }
export interface McpPolicy { mode: "read_only" | "allow_write"; confirm_destructive: boolean; confirm_writes: boolean }
export interface Settings { mirror: string; mirror_rules: MirrorRule[]; proxy: string; install_via?: Record<string, string>; mcp?: McpPolicy }
export interface ShellInfo { shell: string; profile: string; configured: boolean }
export interface Overview {
  home: string;
  cli_path: string;
  cli_found: boolean;
  project_dir: string;
  platform: string;
  settings: Settings;
  tools: ToolStatus[];
  shells: ShellInfo[];
  warnings: string[];
  is_windows: boolean;
  default_shell: string;
  hook_active: boolean;
  app_version: string;
}
export interface Usage { tool: string; version: string; bytes: number }
export interface RemoteVersion { version: string; lts: boolean; prerelease: boolean }
export interface InstallResult { version: string; via: string }
export interface InstallerOption { via: string; available: boolean; detail: string }
export interface InstallerChoice { tool: string; chosen: string; preference: string; options: InstallerOption[] }
export interface Method {
  manager: string;
  method: string;
  program: string;
  args: string[];
  display: string;
  rc_note?: string | null;
  note?: string | null;
}
export interface RegisteredRuntime { tool: string; version: string; global: boolean }
export interface ManagerInfo {
  name: string;
  tools: string[];
  supported: boolean;
  installed: boolean;
  version: string | null;
  entry: string | null;
  data_dir: string | null;
  origin: string;
  runtimes: RegisteredRuntime[];
  install_method: Method | null;
}
export interface RcEdit { file: string; lines: [number, string][] }
export type ManagerAction = "upgrade" | "uninstall";
export interface ManagerPlan {
  manager: string;
  action: ManagerAction;
  kind: "run" | "refuse";
  origin: string;
  command: string[] | null;
  env: [string, string][];
  command_line: string | null;
  remove_dirs: string[];
  rc_edits: RcEdit[];
  runtimes: RegisteredRuntime[];
  data_runtimes: string[];
  keep_data: boolean;
  summary: string;
}
export interface ExecReport { unregistered: RegisteredRuntime[]; notices: string[] }
export type UninstallMode = "auto" | "forget" | "purge";
export interface UninstallPlan {
  tool: string;
  version: string;
  kind: "delete_install" | "forget" | "provider_command" | "remove_dir" | "refuse";
  provider: string | null;
  command: string[] | null;
  env: [string, string][];
  command_line: string | null;
  paths: string[];
  home: string | null;
  needs_root: boolean;
  summary: string;
}
export interface Detected {
  tool: string;
  version: string;
  exe: string;
  found_in: string;
  home: string;
  via: string;
  provider: string;
  formula: string | null;
  adopted: boolean;
  adopted_as: string | null;
  disabled: boolean;
  system_default: boolean;
  runa_global: boolean;
  view_only: boolean;
}
export interface DetectReport { items: Detected[]; from_cache: boolean; stale: boolean; created_at: number }
export interface MigrationAction {
  tool: string;
  version: string;
  action: string;
  old_origin: string | null;
  home: string | null;
  provider: string | null;
  removes: string[];
  note: string;
}
export interface AdoptResult { version: string; set_global: boolean }
export interface AdoptOutcome { tool: string; version: string; home: string; adopted_as: string | null; set_global: boolean; error: string | null }

export interface McpClient {
  id: string;
  name: string;
  method: "deeplink" | "file" | "cli";
  config_path: string | null;
  added: boolean;
  snippet: string;
  link: string | null;
  cli: string | null;
  command_line: string | null;
}
export interface McpStatus { cli_path: string; cli_found: boolean; policy: McpPolicy; bridge_running: boolean; bridge_error: string | null; clients: McpClient[] }
export interface McpOperation { kind: "install" | "set_global" | "uninstall"; tool: string; version: string; via: string | null; mode: string | null }
export interface McpApproval { id: string; op: McpOperation; summary: string; details: string[]; client: string | null }

export type ProjectStatus = "ok" | "mismatch" | "missing" | "system";
export interface ProjectItem { tool: string; required: string; source: string; file: string | null; status: ProjectStatus; resolved: string | null; installed: string[] }
export interface ProjectDetail { path: string; name: string; exists: boolean; items: ProjectItem[] }

/** `install-progress` payloads (core `Progress`, tagged by `stage`). */
export type Progress =
  | { stage: "resolving"; tool: string; spec: string }
  | { stage: "downloading"; tool: string; version: string; done: number; total: number | null }
  | { stage: "verifying"; tool: string; version: string; checksum: boolean }
  | { stage: "extracting"; tool: string; version: string }
  | { stage: "done"; tool: string; version: string }
  | { stage: "warning"; message: string }
  | { stage: "manager"; tool: string; version: string; via: string; command: string }
  | { stage: "output"; tool: string; line: string };
/** `manager-progress` payload: one output line of a manager install / upgrade / uninstall. */
export interface ManagerLine { name: string; line: string }

// ---------------------------------------------------------------- transport
/** Running inside Tauri? Otherwise (plain browser, `npm run dev`) the IPC mock is used. */
export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return inTauri ? tauriInvoke<T>(cmd, args) : mockInvoke<T>(cmd, args);
}
function on<T>(event: string, f: (payload: T) => void): Promise<UnlistenFn> {
  return inTauri ? tauriListen<T>(event, (e) => f(e.payload)) : mockListen<T>(event, f);
}

// ---------------------------------------------------------------- commands
export const api = {
  overview: (project: string | null) => call<Overview>("overview", { project }),
  diskUsage: () => call<Usage[]>("disk_usage"),
  remoteVersions: (tool: string) => call<RemoteVersion[]>("remote_versions", { tool }),
  install: (tool: string, spec: string, via?: string | null) => call<InstallResult>("install", { tool, spec, via: via ?? null }),
  installerChoice: (tool: string) => call<InstallerChoice>("installer_choice", { tool }),
  setInstallVia: (tool: string, via: string) => call<InstallerChoice>("set_install_via", { tool, via }),
  managerInstallPlan: (name: string) => call<Method>("manager_install_plan", { name }),
  /** Install a manager and make it the installer of `tool`. */
  installManager: (name: string, tool: string) => call<InstallerChoice>("install_manager", { name, tool }),
  /** Install a manager without touching any installer preference (管理器 page). */
  installManagerOnly: (name: string) => call<void>("install_manager_only", { name }),
  managerInfo: () => call<ManagerInfo[]>("manager_info"),
  managerPlan: (name: string, action: ManagerAction, keepData = false) => call<ManagerPlan>("manager_plan", { name, action, keepData }),
  managerExecute: (name: string, action: ManagerAction, keepData = false) => call<ExecReport>("manager_execute", { name, action, keepData }),
  uninstallPlan: (tool: string, version: string, mode: UninstallMode) => call<UninstallPlan>("uninstall_plan", { tool, version, mode }),
  uninstall: (tool: string, version: string, mode: UninstallMode) => call<void>("uninstall", { tool, version, mode }),
  setDisabled: (tool: string, version: string, disabled: boolean, project: string | null) =>
    call<string[]>("set_disabled", { tool, version, disabled, project }),
  setGlobal: (tool: string, version: string) => call<string>("set_global", { tool, version }),
  setLocal: (project: string, tool: string, version: string) => call<string>("set_local", { project, tool, version }),
  unsetLocal: (project: string, tool: string) => call<void>("unset_local", { project, tool }),
  detectCached: () => call<DetectReport>("detect_cached"),
  detectRefresh: () => call<DetectReport>("detect_refresh"),
  detectCheck: () => call<boolean>("detect_check"),
  migrateAdoptions: (dryRun: boolean) => call<MigrationAction[]>("migrate_adoptions", { dryRun }),
  adopt: (item: Detected, name: string | null = null) => call<AdoptResult>("adopt", { item, name }),
  adoptAll: () => call<AdoptOutcome[]>("adopt_all"),
  setMirror: (mode: string) => call<Settings>("set_mirror", { mode }),
  setProxy: (value: string) => call<Settings>("set_proxy", { value }),
  setupShell: (shell: string) => call<string>("setup_shell", { shell }),
  removeShell: (shell: string) => call<string | null>("remove_shell", { shell }),
  addUserPath: () => call<boolean>("add_user_path"),
  reshim: () => call<number>("reshim"),
  projectsList: () => call<string[]>("projects_list"),
  projectAdd: (paths: string[]) => call<string[]>("project_add", { paths }),
  projectRemove: (path: string) => call<string[]>("project_remove", { path }),
  projectDetail: (path: string) => call<ProjectDetail>("project_detail", { path }),
  projectSetVersion: (path: string, tool: string, version: string) => call<string>("project_set_version", { path, tool, version }),
  projectIsProject: (path: string) => call<boolean>("project_is_project", { path }),
  projectScan: (root: string) => call<string[]>("project_scan", { root }),
  mcpStatus: () => call<McpStatus>("mcp_status"),
  mcpSetPolicy: (policy: McpPolicy) => call<McpPolicy>("mcp_set_policy", { policy }),
  mcpAdd: (id: string) => call<string>("mcp_add", { id }),
  mcpRemove: (id: string) => call<string>("mcp_remove", { id }),
  mcpPending: () => call<McpApproval[]>("mcp_pending"),
  mcpRespond: (id: string, approved: boolean) => call<void>("mcp_respond", { id, approved }),
};

// ---------------------------------------------------------------- events
export const events = {
  installProgress: (f: (p: Progress) => void) => on<Progress>("install-progress", f),
  managerProgress: (f: (p: ManagerLine) => void) => on<ManagerLine>("manager-progress", f),
  detectUpdated: (f: (r: DetectReport) => void) => on<DetectReport>("detect-updated", f),
  mcpApproval: (f: (r: McpApproval) => void) => on<McpApproval>("mcp-approval", f),
  mcpApprovalDone: (f: (id: string) => void) => on<string>("mcp-approval-done", f),
};
