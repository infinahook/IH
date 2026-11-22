import { invoke } from "@tauri-apps/api/core";
import type { LineDiff } from "./diff";
import type { Locale } from "./i18n";

// ---------------------------------------------------------------- erreurs

export interface SerializedIhError {
  code: string;
  message: string;
  fixable: boolean;
}

/** Erreur IPC normalisée : conserve le code stable, le message brut et le drapeau `fixable`. */
export class AppError extends Error {
  readonly code: string;
  readonly fixable: boolean;
  readonly raw: unknown;

  constructor(code: string, message: string, fixable: boolean, raw: unknown = null) {
    super(message);
    this.name = "AppError";
    this.code = code;
    this.fixable = fixable;
    this.raw = raw;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function toAppError(error: unknown): AppError {
  if (error instanceof AppError) {
    return error;
  }
  if (isRecord(error) && typeof error.message === "string") {
    return new AppError(
      typeof error.code === "string" && error.code.length > 0 ? error.code : "unknown",
      error.message,
      error.fixable === true,
      error,
    );
  }
  if (error instanceof Error) {
    return new AppError("unknown", error.message, false, error);
  }
  if (typeof error === "string") {
    return new AppError("unknown", error, false, error);
  }
  let serialized = "Erreur inconnue";
  try {
    serialized = JSON.stringify(error) ?? serialized;
  } catch {
    serialized = String(error);
  }
  return new AppError("unknown", serialized, false, error);
}

/** Appelle une commande Tauri et convertit tout rejet en `AppError`. */
export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toAppError(error);
  }
}

// ------------------------------------------------------------- types noyau

export type AgentState =
  | "starting"
  | "ready"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "cancelled";

export type TaskState =
  | "pending"
  | "ready"
  | "running"
  | "blocked"
  | "completed"
  | "failed"
  | "cancelled";

export type RiskLevel = "low" | "medium" | "high" | "critical";
export type ApprovalState = "pending" | "approved" | "denied" | "consumed" | "expired";
export type ApprovalDecision = "approve" | "deny";
export type ProviderKind = "openai_compatible" | "anthropic" | "google" | "ollama";
export type AgentKind = "claude_code" | "codex" | "gemini_cli" | "opencode" | "custom";
export type DistroFamily = "arch" | "debian" | "other";
export type ThemeName = "dark" | "light" | "system";
export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

export interface Health {
  app: string;
  version: string;
  platform: string;
  locale: string;
  core_online: boolean;
}

export interface PlatformInfo {
  os_id: string;
  os_name: string;
  os_version: string;
  family: DistroFamily;
  arch: string;
  kernel: string;
  cpu_cores: number;
  ram_mb: number;
  gpu: string | null;
  recommended_max_agents: number;
}

export interface PathsInfo {
  root: string;
  config_file: string;
  database: string;
  logs: string;
  skills: string;
  plugins: string;
}

export type SecretStoreStatus =
  | { state: "available"; backend: string }
  | { state: "unavailable"; reason: string };

export interface CommandAssessment {
  risk: RiskLevel;
  reasons: string[];
  requires_approval: boolean;
}

export interface ApprovalRequest {
  id: string;
  category: string;
  summary: string;
  detail: string;
  risk: RiskLevel;
  created_at: string;
  expires_at: string;
  state: ApprovalState;
}

export interface DiagnosticsReport {
  generated_at: string;
  platform: PlatformInfo;
  app_version: string;
  locale: string;
  projects: number;
  threads: number;
  tasks: number;
  memory_entries: number;
  installed_skills: number;
  installed_plugins: number;
  database: string;
  log_tail: string[];
}

// ------------------------------------------------------------ configuration

export interface ModelEntry {
  id: string;
  label: string;
  context_window: number | null;
  supports_tools: boolean;
  supports_vision: boolean;
  local: boolean;
}

export interface ProviderConfig {
  id: string;
  kind: ProviderKind;
  label: string;
  base_url: string;
  enabled: boolean;
  default_model: string | null;
  models: ModelEntry[];
  has_secret: boolean;
  timeout_secs: number;
}

export interface AgentConfig {
  id: string;
  kind: AgentKind;
  label: string;
  program: string;
  args: string[];
  enabled: boolean;
  env: Record<string, string>;
  timeout_secs: number;
}

export interface UpdateConfig {
  channel: string;
  endpoint: string | null;
  check_interval_minutes: number;
  auto_download: boolean;
  public_key_hex: string | null;
  allow_prerelease: boolean;
}

export interface TerminalConfig {
  shell: string;
  args: string[];
}

export interface UiConfig {
  sidebar_collapsed: boolean;
  editor_font_size: number;
  reduce_motion: boolean;
}

export interface LoggingConfig {
  level: string;
  retention_days: number;
  json: boolean;
}

export interface ResourcePolicy {
  max_concurrent_agents: number;
  max_output_bytes_per_session: number;
  transcript_page_size: number;
  terminal_scrollback_lines: number;
  preload_local_models: boolean;
  embedding_cache_mb: number;
}

export interface AppConfig {
  schema: number;
  locale: string;
  theme: string;
  resources: ResourcePolicy;
  providers: ProviderConfig[];
  agents: AgentConfig[];
  updates: UpdateConfig;
  terminal: TerminalConfig;
  ui: UiConfig;
  logging: LoggingConfig;
  recent_projects: string[];
  last_project: string | null;
  telemetry: boolean;
}

// ---------------------------------------------------------------- projets

export interface ProjectView {
  id: string;
  name: string;
  path: string;
  is_git: boolean;
  last_opened_at: string;
}

export interface FileNode {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
}

export interface FileContent {
  path: string;
  content: string;
  truncated: boolean;
  size: number;
}

export interface SearchHit {
  path: string;
  line: number;
  column: number;
  preview: string;
}

// ------------------------------------------------------------------- git

export interface StatusEntry {
  path: string;
  index: string;
  worktree: string;
  staged: boolean;
}

export interface GitStatus {
  branch: string;
  entries: StatusEntry[];
  ahead: number;
  behind: number;
  clean: boolean;
}

export interface CommitInfo {
  hash: string;
  author: string;
  date: string;
  subject: string;
}

export interface Worktree {
  path: string;
  head: string;
  branch: string | null;
  bare: boolean;
}

// --------------------------------------------------------------- agents

export interface AgentInfo {
  id: string;
  label: string;
  kind: AgentKind;
  program: string;
  enabled: boolean;
  detected: boolean;
  resolved_path: string | null;
}

export interface SessionSnapshot {
  session_id: string;
  agent_id: string;
  state: AgentState;
  exit_code: number | null;
  bytes_emitted: number;
}

export interface SessionRow {
  id: string;
  project_id: string;
  agent_id: string;
  thread_id: string | null;
  state: AgentState;
  exit_code: number | null;
  created_at: string;
  ended_at: string | null;
}

// ---------------------------------------------------------- fournisseurs

export interface ProviderView {
  id: string;
  label: string;
  kind: ProviderKind;
  base_url: string;
  enabled: boolean;
  has_secret: boolean;
  secret_stored: boolean;
  default_model: string | null;
  models: ModelEntry[];
}

export interface HealthReport {
  ok: boolean;
  latency_ms: number;
  detail: string;
}

// ------------------------------------------------------ skills et MCP

export interface SkillView {
  id: string;
  name: string;
  version: string;
  source: string;
  enabled: boolean;
  sha256: string | null;
  verified: boolean | null;
  dir: string;
}

export interface InstallReport {
  id: string;
  version: string;
  sha256: string;
  files: number;
}

export interface InstalledExtension {
  id: string;
  kind: string;
  name: string;
  version: string;
  source: string;
  manifest: unknown;
  sha256: string | null;
  enabled: boolean;
  installed_at: string;
  updated_at: string;
}

export interface McpTool {
  name: string;
  description: string;
  inputSchema: unknown;
}

export interface McpCallOutcome {
  status: string;
  approval_id?: string | null;
  /** Variante camelCase acceptée si le noyau renvoie le champ sous cette forme. */
  approvalId?: string | null;
  result: unknown;
}

/** Identifiant d'approbation d'un résultat MCP, quel que soit le casing du noyau. */
export function outcomeApprovalId(outcome: McpCallOutcome): string | null {
  return outcome.approval_id ?? outcome.approvalId ?? null;
}

// ------------------------------------------ tâches, mémoire, historique

export interface Task {
  id: string;
  project_id: string;
  title: string;
  description: string;
  state: TaskState;
  assignee: string | null;
  depends_on: string[];
  role: string;
  created_at: string;
  updated_at: string;
}

export interface PlanView {
  goal: string;
  tasks: Task[];
}

export interface MemoryEntry {
  id: string;
  project_id: string | null;
  scope: string;
  key: string | null;
  content: string;
  tags: string[];
  source: string;
  created_at: string;
  updated_at: string;
}

export interface Thread {
  id: string;
  project_id: string;
  title: string;
  agent_id: string | null;
  created_at: string;
  updated_at: string;
}

export interface Message {
  id: string;
  thread_id: string;
  role: string;
  content: string;
  metadata: unknown;
  created_at: string;
}

export interface ChatUsage {
  input_tokens: number | null;
  output_tokens: number | null;
}

/** Accusé de réception immédiat de `chat_send` : le flux arrive ensuite par événements. */
export interface ChatAccepted {
  request_id: string;
  thread_id: string;
  project_id: string;
  provider_id: string;
  model: string;
}

/** Résultat final d'un tour de conversation, y compris annulé. */
export interface ChatOutcome {
  user_message_id: string;
  assistant_message_id: string;
  text: string;
  usage: ChatUsage;
  cancelled: boolean;
  finish_reason: string | null;
  elapsed_ms: number;
}

export interface Workflow {
  id: string;
  project_id: string;
  name: string;
  interval_minutes: number;
  enabled: boolean;
  steps: unknown;
  last_run_at: string | null;
  created_at: string;
}

export interface WorkflowRun {
  id: number;
  workflow_id: string;
  status: string;
  detail: string | null;
  started_at: string;
  finished_at: string | null;
}

// -------------------------------------------------------------- workspace

/** Limites et exclusions du contexte projet, telles que le noyau les applique. */
export interface ContextSettings {
  max_files: number;
  max_bytes_per_file: number;
  max_total_bytes: number;
  exclusions: string[];
  /**
   * Limite de l'éditeur. Le contrat actuel ne l'expose pas toujours ; quand
   * elle est absente, l'interface retombe sur une borne prudente et sur le
   * drapeau `truncated` de `file_read`.
   */
  max_editor_bytes?: number;
}

/** Métadonnées disque d'un fichier du projet (empreinte, binaire, taille). */
export interface WorkspaceMeta {
  path: string;
  size: number;
  modified_ms: number | null;
  sha256: string;
  is_binary: boolean;
}

/** Résultat d'une écriture vérifiée. */
export interface WorkspaceWriteOutcome {
  bytes: number;
  sha256: string;
  backup: string | null;
}

/** Candidat de la liste de mentions `@` (`files_match`). */
export interface FileMatch {
  path: string;
  score: number;
  is_dir: boolean;
  excluded: boolean;
}

export interface PreparedContextFile {
  path: string;
  content: string;
  bytes: number;
  truncated: boolean;
  sensitive: boolean;
}

export interface SkippedContextFile {
  path: string;
  reason: string;
}

/** Aperçu exact de ce que `chat_send` joindra au prochain message. */
export interface PreparedContext {
  files: PreparedContextFile[];
  skipped: SkippedContextFile[];
  total_bytes: number;
  prompt_block: string;
}

/** Diff préparé côté noyau, en attente d'acceptation ou de rejet. */
export interface DiffPreview {
  id: string;
  path: string;
  before_sha256: string | null;
  additions: number;
  deletions: number;
  diff: LineDiff;
}

/** Modification préparée mais non appliquée. */
export interface PendingDiff {
  id: string;
  project_id: string;
  path: string;
  created_at: string;
}

// ------------------------------------------------------------ mises à jour

export interface UpdateStatus {
  state: string;
  version: string | null;
  notes: string | null;
  progress: number;
  error: string | null;
}

// ---------------------------------------------------------------- API

const id = (projectId: string) => ({ projectId });

export const api = {
  // noyau
  health: () => call<Health>("app_health"),
  platformInfo: () => call<PlatformInfo>("platform_info"),
  configGet: () => call<AppConfig>("config_get"),
  configUpdate: (config: AppConfig) => call<AppConfig>("config_update", { patch: config }),
  localeSet: (locale: Locale) => call<AppConfig>("locale_set", { locale }),
  pathsInfo: () => call<PathsInfo>("paths_info"),
  secretStoreStatus: () => call<SecretStoreStatus>("secret_store_status"),
  classifyCommand: (command: string) =>
    call<CommandAssessment>("classify_command_preview", { command }),
  approvalsPending: () => call<ApprovalRequest[]>("approvals_pending"),
  approvalResolve: (approvalId: string, decision: ApprovalDecision) =>
    call<ApprovalRequest>("approval_resolve", { id: approvalId, decision }),
  openExternal: (url: string) => call<void>("open_external", { url }),
  openPath: (path: string) => call<void>("open_path", { path }),
  logTail: (lines: number) => call<string[]>("log_tail", { lines }),
  diagnosticsReport: () => call<DiagnosticsReport>("diagnostics_report"),
  appVersion: () => call<string>("app_version"),

  // projets et fichiers
  projects: {
    add: (path: string) => call<ProjectView>("project_add", { path }),
    list: () => call<ProjectView[]>("project_list"),
    remove: (projectId: string) => call<void>("project_remove", { id: projectId }),
  },
  files: {
    tree: (projectId: string, depth: number) =>
      call<FileNode[]>("file_tree", { projectId, depth }),
    read: (projectId: string, path: string, maxBytes: number | null = null) =>
      call<FileContent>("file_read", { projectId, path, maxBytes }),
    write: (projectId: string, path: string, content: string) =>
      call<number>("file_write", { projectId, path, content }),
    search: (projectId: string, query: string, limit = 100) =>
      call<SearchHit[]>("file_search", { projectId, query, limit }),
  },

  // git
  git: {
    status: (projectId: string) => call<GitStatus>("git_status", id(projectId)),
    diff: (projectId: string, staged: boolean, path: string | null = null) =>
      call<string>("git_diff", { projectId, staged, path }),
    stage: (projectId: string, paths: string[]) =>
      call<void>("git_stage", { projectId, paths }),
    unstage: (projectId: string, paths: string[]) =>
      call<void>("git_unstage", { projectId, paths }),
    commit: (projectId: string, message: string) =>
      call<string>("git_commit", { projectId, message }),
    branches: (projectId: string) => call<string[]>("git_branches", id(projectId)),
    log: (projectId: string, limit = 30) =>
      call<CommitInfo[]>("git_log", { projectId, limit }),
    worktrees: (projectId: string) => call<Worktree[]>("git_worktrees", id(projectId)),
    addWorktree: (projectId: string, target: string, branch: string, create: boolean) =>
      call<void>("git_add_worktree", { projectId, target, branch, create }),
    removeWorktree: (projectId: string, target: string, force: boolean) =>
      call<void>("git_remove_worktree", { projectId, target, force }),
  },

  // terminal
  terminal: {
    open: (projectId: string, cols: number, rows: number) =>
      call<string>("terminal_open", { projectId, cols, rows }),
    write: (terminalId: string, data: string) =>
      call<void>("terminal_write", { id: terminalId, data }),
    resize: (terminalId: string, cols: number, rows: number) =>
      call<void>("terminal_resize", { id: terminalId, cols, rows }),
    close: (terminalId: string) => call<void>("terminal_close", { id: terminalId }),
  },

  // agents
  agents: {
    list: () => call<AgentInfo[]>("agents_list"),
    start: (projectId: string, agentId: string, prompt: string) =>
      call<string>("agent_start", { projectId, agentId, prompt }),
    cancel: (sessionId: string) => call<void>("agent_cancel", { sessionId }),
    state: (sessionId: string) =>
      call<SessionSnapshot | null>("agent_state", { sessionId }),
    sessions: (projectId: string) =>
      call<SessionRow[]>("agent_sessions", id(projectId)),
  },

  // fournisseurs
  providers: {
    list: () => call<ProviderView[]>("providers_list"),
    setSecret: (providerId: string, secret: string) =>
      call<void>("provider_set_secret", { providerId, secret }),
    clearSecret: (providerId: string) =>
      call<void>("provider_clear_secret", { providerId }),
    health: (providerId: string) => call<HealthReport>("provider_health", { providerId }),
    models: (providerId: string) => call<ModelEntry[]>("models_list", { providerId }),
    setDefaultModel: (providerId: string, model: string) =>
      call<void>("provider_set_default_model", { providerId, model }),
    setEnabled: (providerId: string, enabled: boolean) =>
      call<void>("provider_set_enabled", { providerId, enabled }),
  },

  // skills
  skills: {
    list: () => call<SkillView[]>("skills_list"),
    install: (path: string) => call<InstallReport>("skill_install", { path }),
    verify: (skillId: string) => call<boolean>("skill_verify", { id: skillId }),
    remove: (skillId: string, version: string) =>
      call<void>("skill_remove", { id: skillId, version }),
    setEnabled: (skillId: string, enabled: boolean) =>
      call<void>("skill_set_enabled", { id: skillId, enabled }),
  },

  // MCP
  mcp: {
    list: () => call<InstalledExtension[]>("mcp_servers_list"),
    add: (serverId: string, name: string, command: string, args: string[]) =>
      call<void>("mcp_server_add", { id: serverId, name, command, args }),
    remove: (serverId: string) => call<void>("mcp_server_remove", { id: serverId }),
    connect: (serverId: string, clientName = "IH") =>
      call<McpTool[]>("mcp_connect", { id: serverId, clientName }),
    callTool: (
      serverId: string,
      tool: string,
      args: Record<string, unknown>,
      approvalId: string | null = null,
    ) =>
      call<McpCallOutcome>("mcp_call_tool", {
        id: serverId,
        tool,
        arguments: args,
        approvalId,
      }),
    disconnect: (serverId: string) => call<void>("mcp_disconnect", { id: serverId }),
  },

  // tâches
  tasks: {
    list: (projectId: string) => call<Task[]>("tasks_list", id(projectId)),
    create: (
      projectId: string,
      title: string,
      description: string | null = null,
      role: string | null = null,
      assignee: string | null = null,
    ) => call<Task>("task_create", { projectId, title, description, role, assignee }),
    setState: (taskId: string, taskState: TaskState) =>
      call<void>("task_set_state", { id: taskId, taskState }),
    plan: (projectId: string, goal: string, workers: number) =>
      call<PlanView>("plan_create", { projectId, goal, workers }),
  },

  // mémoire
  memory: {
    list: (projectId: string | null) => call<MemoryEntry[]>("memory_list", { projectId }),
    add: (projectId: string | null, content: string, tags: string[] = []) =>
      call<MemoryEntry>("memory_add", { projectId, content, tags }),
    search: (projectId: string | null, query: string) =>
      call<MemoryEntry[]>("memory_search", { projectId, query }),
    remove: (entryId: string) => call<void>("memory_delete", { id: entryId }),
    preview: (projectId: string | null) => call<string>("memory_preview", { projectId }),
  },

  // conversation : le flux réel arrive par les événements `ih://chat-*`
  chat: {
    send: (
      projectId: string,
      threadId: string | null,
      providerId: string,
      model: string,
      prompt: string,
      system: string | null = null,
      historyLimit: number | null = null,
      contextPaths: string[] = [],
      allowSensitiveContext = false,
    ) =>
      call<ChatAccepted>("chat_send", {
        projectId,
        threadId,
        providerId,
        model,
        prompt,
        system,
        historyLimit,
        // Le noyau lit lui-même les fichiers : seuls les chemins transitent ici.
        contextPaths,
        allowSensitiveContext,
      }),
    cancel: (requestId: string) => call<boolean>("chat_cancel", { requestId }),
    active: () => call<number>("chat_active"),
  },

  // historique
  history: {
    threads: (projectId: string) => call<Thread[]>("threads_list", id(projectId)),
    search: (projectId: string, query: string, limit = 50) =>
      call<Thread[]>("threads_search", { projectId, query, limit }),
    createThread: (projectId: string, title: string) =>
      call<Thread>("thread_create", { projectId, title }),
    renameThread: (threadId: string, title: string) =>
      call<void>("thread_rename", { id: threadId, title }),
    messages: (threadId: string, limit: number, offset: number) =>
      call<Message[]>("messages_list", { threadId, limit, offset }),
    deleteThread: (threadId: string) =>
      call<void>("thread_delete", { id: threadId }),
    exportProject: (projectId: string) =>
      call<string>("history_export", { projectId }),
  },

  // automatisations
  workflows: {
    list: (projectId: string) => call<Workflow[]>("workflows_list", id(projectId)),
    upsert: (
      projectId: string,
      name: string,
      intervalMinutes: number,
      steps: unknown,
    ) => call<Workflow>("workflow_upsert", { projectId, name, intervalMinutes, steps }),
    runNow: (workflowId: string) =>
      call<WorkflowRun>("workflow_run_now", { id: workflowId }),
    runs: (workflowId: string, limit = 20) =>
      call<WorkflowRun[]>("workflow_runs", { id: workflowId, limit }),
  },

  // espace de travail : fichiers, contexte, diff et disposition
  workspace: {
    contextSettings: () => call<ContextSettings>("context_settings"),
    meta: (projectId: string, path: string) =>
      call<WorkspaceMeta>("workspace_meta", { projectId, path }),
    save: (
      projectId: string,
      path: string,
      content: string,
      expectedSha256: string | null = null,
    ) =>
      call<WorkspaceWriteOutcome>("workspace_save", {
        projectId,
        path,
        content,
        expectedSha256,
      }),
    match: (projectId: string, query: string, limit = 20) =>
      call<FileMatch[]>("files_match", { projectId, query, limit }),
    prepareContext: (projectId: string, paths: string[], allowSensitive = false) =>
      call<PreparedContext>("context_prepare", { projectId, paths, allowSensitive }),
    diffPrepare: (projectId: string, path: string, content: string) =>
      call<DiffPreview>("diff_prepare", { projectId, path, content }),
    diffPending: () => call<PendingDiff[]>("diff_pending"),
    diffAccept: (diffId: string) =>
      call<WorkspaceWriteOutcome>("diff_accept", { id: diffId }),
    diffReject: (diffId: string) => call<void>("diff_reject", { id: diffId }),
    layoutGet: () => call<string | null>("layout_get"),
    layoutSave: (layout: string) => call<void>("layout_save", { layout }),
  },

  // mises à jour
  updates: {
    check: () => call<UpdateStatus>("update_check"),
    status: () => call<UpdateStatus>("update_status"),
    download: () => call<UpdateStatus>("update_download"),
    plan: () => call<string>("update_plan"),
    requestApproval: () => call<string>("update_request_approval"),
    apply: (approvalId: string) => call<string>("update_apply", { approvalId }),
  },
} as const;

export type Api = typeof api;
