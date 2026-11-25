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
export type AgentKind =
  | "claude_code"
  | "codex"
  | "gemini_cli"
  | "opencode"
  | "pi"
  | "kilo"
  | "dsh"
  | "goose"
  | "custom";
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
  endpoint_path: string | null;
  headers: Record<string, string>;
  streaming: boolean;
  /** Capacité tool calling natif déclarée pour ce fournisseur. */
  supports_tools: boolean;
  temperature: number | null;
  max_tokens: number | null;
  custom: boolean;
  created_at: string | null;
  updated_at: string | null;
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
  /** Version réellement lue par `<exécutable> --version`. */
  version: string | null;
  /** Diagnostic quand la détection ou la sonde de version échoue. */
  error: string | null;
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
  /** Vrai pour un fournisseur ajouté par l'utilisateur, faux pour un intégré. */
  custom: boolean;
  endpoint_path: string | null;
  /** Nombre d'en-têtes configurés ; les valeurs ne sont jamais renvoyées. */
  headers_count: number;
  streaming: boolean;
  /** Capacité tool calling natif déclarée pour ce fournisseur. */
  supports_tools: boolean;
  timeout_secs: number;
  created_at: string | null;
  updated_at: string | null;
}

/** Saisie de `provider_create` / `provider_update`, sérialisée telle quelle par Rust. */
export interface ProviderInput {
  id: string;
  label: string;
  kind: ProviderKind;
  base_url: string;
  endpoint_path?: string | null;
  headers?: Record<string, string>;
  timeout_secs?: number;
  streaming?: boolean;
  /** Déclaration explicite du tool calling natif (absente : inchangée). */
  supports_tools?: boolean;
  temperature?: number | null;
  max_tokens?: number | null;
  default_model?: string | null;
  models?: ModelEntry[];
  /** Transmise une seule fois : le noyau la range dans le trousseau. */
  api_key?: string | null;
}

/** Résultat de `provider_test`. */
export interface ConnectionTest {
  status: string;
  detail: string;
  latency_ms: number;
  models: number;
  protocol: string;
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
  /** Vrai si le fournisseur accepte les cartes d'appel d'outils natives. */
  native_tools: boolean;
}

/** Appel d'outil demandé par le modèle (identifiant du fournisseur). */
export interface ChatToolCallRequest {
  id: string;
  name: string;
  arguments: unknown;
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
  tool_calls?: ChatToolCallRequest[];
}

/** Proposition d'appel d'outil persistée (arguments déjà masqués). */
export interface ChatToolCallView {
  id: string;
  request_id: string;
  round: number;
  provider_call_id: string;
  exposed_name: string;
  server_id: string;
  tool: string;
  risk: string;
  category: string;
  arguments: unknown;
  status: string;
  error: string | null;
  result_text: string | null;
  created_at: string;
  decided_at: string | null;
  finished_at: string | null;
}

/** Résultat d'une décision humaine sur une carte d'appel. */
export interface ChatToolDecisionOutcome {
  call_id: string;
  status: string;
  summary: string;
  result_text: string | null;
  continued: boolean;
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

// ------------------------------------------------------------------ MCP

export interface McpRemoteServer {
  id: string;
  name: string;
  url: string;
  transport: string;
  timeout_secs: number;
  enabled: boolean;
  allow_auto_approve: boolean;
  has_secret: boolean;
  last_status: string;
  last_error: string | null;
  last_connected_at: string | null;
  created_at: string;
  updated_at: string;
  tool_count: number;
  connected: boolean;
}

export interface McpRemoteTool {
  server_id: string;
  name: string;
  description: string;
  input_schema: unknown;
  output_schema?: unknown;
  category: string;
  risk: string;
  enabled: boolean;
  discovered_at: string;
}

export interface McpRemoteToolResult {
  content: unknown[];
  text: string;
  structured?: unknown;
  is_error: boolean;
  truncated: boolean;
}

export interface McpRemoteCallOutcome {
  status: string;
  approval_id: string | null;
  fingerprint: string | null;
  result: McpRemoteToolResult | null;
}

export interface McpTestOutcome {
  ok: boolean;
  tool_count: number;
  error: string | null;
}

export interface McpCallRecord {
  id: number;
  server_id: string;
  tool_name: string;
  caller: string;
  caller_id: string | null;
  arguments: unknown;
  status: string;
  error: string | null;
  duration_ms: number | null;
  started_at: string;
  finished_at: string | null;
}

export interface McpServerUpsert {
  serverId?: string;
  name: string;
  url: string;
  transport: string;
  headers?: Record<string, string>;
  timeoutSecs: number;
  enabled: boolean;
  allowAutoApprove: boolean;
  token?: string;
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
  /** `editor` ou `agent:<session_id>` : origine réelle de la modification. */
  origin: string;
  /** Vrai si l'acceptation supprime le fichier. */
  delete: boolean;
  created_at: string;
}

// ------------------------------------------------------------- mode agent

/** Adaptateur d'agent tel que `agent_adapters` le rapporte. */
export interface AgentAdapterReport {
  id: string;
  label: string;
  kind: string;
  program: string;
  detected: boolean;
  path: string | null;
  version: string | null;
  /** Exécutable trouvé **et** répondant correctement à `--version`. */
  functional: boolean;
  capabilities: string[];
  diagnostic: string;
}

/** Demande d'approbation d'un lancement de session (`agent_session_request_approval`). */
export interface AgentApprovalView {
  approval_id: string;
  agent_id: string;
  label: string;
  command: string;
  workdir: string;
  risk: RiskLevel;
  fingerprint: string;
  detail: string;
}

/** Accusé de réception de `agent_session_start`. */
export interface AgentSessionStarted {
  session_id: string;
  workdir: string;
  agent_id: string;
}

/** Session d'agent persistée avec ses métadonnées d'exécution. */
export interface AgentSessionDetail {
  session: SessionRow;
  adapter: string | null;
  task: string | null;
  workdir: string | null;
  summary: string | null;
  files: string[];
}

/** Événement persisté d'une session (`agent_session_events`). */
export interface AgentEventRow {
  id: number;
  session_id: string;
  seq: number;
  kind: string;
  payload: unknown;
  created_at: string;
}

// --------------------------------------------------------------- mode équipe

/**
 * Saisie de `team_request_approval` / `team_run`.
 * Les champs internes restent en `snake_case` : Tauri ne renomme que les
 * arguments de commande, pas les champs d'une structure désérialisée.
 */
export interface TeamRunRequest {
  project_id: string;
  goal: string;
  lead: string;
  workers: string[];
  reviewer: string | null;
  work_items: number;
}

/** Demande d'approbation d'une course d'équipe (`team_request_approval`). */
export interface TeamApprovalView {
  approval_id: string;
  detail: string;
  fingerprint: string;
  plan_summary: string;
}

/** Tâche d'une course, telle que `team_run` la renvoie. */
export interface TeamRunTask {
  task_id: string;
  title: string;
  role: string;
  state: string;
  sessions: number;
}

/** Résultat final de `team_run` (les modifications sont en attente de diff). */
export interface TeamRunResult {
  run_id: string;
  summary: string;
  files: string[];
  changes_prepared: number;
  tasks: TeamRunTask[];
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

  // mode agent (0.2.2) : adaptateurs, sessions isolées, approbations
  agent: {
    adapters: () => call<AgentAdapterReport[]>("agent_adapters"),
    requestApproval: (projectId: string, agentId: string, task: string) =>
      call<AgentApprovalView>("agent_session_request_approval", {
        projectId,
        agentId,
        task,
      }),
    start: (
      projectId: string,
      threadId: string | null,
      agentId: string,
      task: string,
      approvalId: string,
      timeoutSecs: number | null = null,
    ) =>
      call<AgentSessionStarted>("agent_session_start", {
        projectId,
        threadId,
        agentId,
        task,
        approvalId,
        timeoutSecs,
      }),
    cancel: (sessionId: string) =>
      call<void>("agent_session_cancel", { sessionId }),
    refuse: (approvalId: string) =>
      call<ApprovalRequest>("agent_session_refuse", { approvalId }),
    sessions: (projectId: string, limit: number | null = null) =>
      call<AgentSessionDetail[]>("agent_session_list", { projectId, limit }),
    detail: (sessionId: string) =>
      call<AgentSessionDetail>("agent_session_detail", { sessionId }),
    events: (sessionId: string, limit: number | null = null) =>
      call<AgentEventRow[]>("agent_session_events", { sessionId, limit }),
  },

  // mode équipe : une approbation couvre toute la course, les diffs sont
  // ensuite relus via `diff_pending` (origine `team:<run_id>`)
  team: {
    requestApproval: (request: TeamRunRequest) =>
      call<TeamApprovalView>("team_request_approval", { request }),
    run: (request: TeamRunRequest, approvalId: string) =>
      call<TeamRunResult>("team_run", { request, approvalId }),
  },

  // fournisseurs
  providers: {
    list: () => call<ProviderView[]>("providers_list"),
    create: (input: ProviderInput) => call<ProviderView>("provider_create", { input }),
    update: (id: string, input: ProviderInput) =>
      call<ProviderView>("provider_update", { id, input }),
    remove: (id: string) => call<void>("provider_delete", { id }),
    test: (id: string) => call<ConnectionTest>("provider_test", { id }),
    addModel: (providerId: string, modelId: string, label: string | null = null) =>
      call<ProviderView>("provider_add_model", { providerId, modelId, label }),
    removeModel: (providerId: string, modelId: string) =>
      call<ProviderView>("provider_remove_model", { providerId, modelId }),
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
    setEnabled: (serverId: string, enabled: boolean) =>
      call<void>("mcp_stdio_set_enabled", { id: serverId, enabled }),
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

  // serveurs MCP distants (Streamable HTTP / SSE)
  mcpRemote: {
    list: () => call<McpRemoteServer[]>("mcp_remote_servers_list"),
    upsert: (server: McpServerUpsert) =>
      call<McpRemoteServer>("mcp_server_upsert", {
        serverId: server.serverId,
        name: server.name,
        url: server.url,
        transport: server.transport,
        headers: server.headers,
        timeoutSecs: server.timeoutSecs,
        enabled: server.enabled,
        allowAutoApprove: server.allowAutoApprove,
        token: server.token,
      }),
    remove: (serverId: string) => call<void>("mcp_server_delete", { id: serverId }),
    setEnabled: (serverId: string, enabled: boolean) =>
      call<McpRemoteServer>("mcp_server_set_enabled", { id: serverId, enabled }),
    setSecret: (serverId: string, secret: string) =>
      call<void>("mcp_server_set_secret", { id: serverId, secret }),
    clearSecret: (serverId: string) =>
      call<void>("mcp_server_clear_secret", { id: serverId }),
    connect: (serverId: string) =>
      call<McpRemoteTool[]>("mcp_server_connect", { id: serverId }),
    disconnect: (serverId: string) =>
      call<void>("mcp_server_disconnect", { id: serverId }),
    test: (serverId: string) => call<McpTestOutcome>("mcp_server_test", { id: serverId }),
    refreshTools: (serverId: string) =>
      call<McpRemoteTool[]>("mcp_server_refresh_tools", { id: serverId }),
    tools: (serverId?: string) =>
      call<McpRemoteTool[]>("mcp_tools_list", { serverId }),
    setToolEnabled: (serverId: string, name: string, enabled: boolean) =>
      call<McpRemoteTool>("mcp_tool_set_enabled", { serverId, name, enabled }),
    call: (request: {
      serverId: string;
      tool: string;
      arguments: unknown;
      caller?: string;
      callerId?: string;
      approvalId?: string;
      role?: string;
    }) => call<McpRemoteCallOutcome>("mcp_call", {
      serverId: request.serverId,
      tool: request.tool,
      arguments: request.arguments,
      caller: request.caller,
      callerId: request.callerId,
      approvalId: request.approvalId,
      role: request.role,
    }),
    rolePermissions: () => call<Record<string, string[]>>("mcp_role_permissions_get"),
    setRolePermissions: (role: string, tools: string[]) =>
      call<Record<string, string[]>>("mcp_role_permissions_set", { role, tools }),
    calls: (limit = 50, serverId?: string) =>
      call<McpCallRecord[]>("mcp_calls_list", { limit, serverId }),
    exportConfig: () => call<string>("mcp_export_config"),
    agentConfig: (flavor: string) => call<string>("mcp_agent_config", { flavor }),
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
    appendToolResult: (threadId: string, tool: string, status: string, content: string) =>
      call<Message>("chat_append_tool_result", { threadId, tool, status, content }),
    toolCalls: (threadId: string) =>
      call<ChatToolCallView[]>("chat_tool_calls_list", { threadId }),
    toolDecide: (callId: string, decision: "approve" | "deny") =>
      call<ChatToolDecisionOutcome>("chat_tool_decide", { callId, decision }),
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
    diffView: (id: string) => call<DiffPreview>("diff_view", { id }),
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
