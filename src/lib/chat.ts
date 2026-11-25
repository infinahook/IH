import type { ChatOutcome, ChatUsage, Message, Thread } from "./api";

// ------------------------------------------------------- événements Tauri

export interface ChatDeltaEvent {
  request_id: string;
  thread_id: string;
  delta: string;
}

export interface ChatDoneEvent {
  request_id: string;
  thread_id: string;
  outcome: ChatOutcome;
}

export interface ChatErrorEvent {
  request_id: string;
  thread_id: string;
  code: string;
  message: string;
}

/** Proposition d'appel d'outil MCP émise par le modèle (jamais exécutée telle quelle). */
export interface McpToolProposal {
  server_id: string;
  tool: string;
  arguments: unknown;
}

export interface ChatMcpProposalEvent {
  request_id: string;
  thread_id: string;
  proposals: McpToolProposal[];
}

/** Carte d'appel d'outil natif : proposition, décision ou résultat. */
export interface ChatToolCallEvent {
  phase: string;
  thread_id: string;
  request_id: string;
  call_id: string;
  provider_call_id: string;
  round: number;
  exposed_name: string;
  server_id: string;
  tool: string;
  risk: string;
  category: string;
  arguments: unknown;
  status: string;
  error: string | null;
  result_text: string | null;
}

// ------------------------------------------------------ état de streaming

export type ChatStreamStatus = "idle" | "streaming" | "done" | "error";

export interface ChatStreamError {
  code: string;
  message: string;
}

/**
 * État minimal du tour courant. Le texte provisoire est accumulé dans `text`
 * puis remplacé par `outcome.text` à la fin, sans jamais concaténer deux fois
 * le texte final.
 */
export interface ChatStreamState {
  status: ChatStreamStatus;
  requestId: string | null;
  threadId: string | null;
  text: string;
  outcome: ChatOutcome | null;
  error: ChatStreamError | null;
}

const SENSITIVE_KEY = /token|secret|password|passwd|api[-_]?key|authorization|cookie|bearer|credential|private[-_]?key/i;

/**
 * Masque les valeurs des clés sensibles d'un objet JSON d'arguments, pour
 * qu'aucun secret saisi par l'utilisateur n'apparaisse dans une carte d'outil.
 */
export function maskSensitiveArguments(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => maskSensitiveArguments(entry));
  }
  if (typeof value === "object" && value !== null) {
    const masked: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      masked[key] = SENSITIVE_KEY.test(key) ? "[masqué]" : maskSensitiveArguments(entry);
    }
    return masked;
  }
  return value;
}

export const INITIAL_CHAT_STREAM: ChatStreamState = {
  status: "idle",
  requestId: null,
  threadId: null,
  text: "",
  outcome: null,
  error: null,
};

export type ChatStreamAction =
  | { type: "begin"; requestId: string; threadId: string }
  | { type: "delta"; requestId: string; threadId: string; delta: string }
  | { type: "done"; requestId: string; threadId: string; outcome: ChatOutcome }
  | { type: "error"; requestId: string; threadId: string; code: string; message: string }
  | { type: "reset" };

function isCurrent(state: ChatStreamState, requestId: string, threadId: string): boolean {
  return (
    state.status === "streaming" &&
    state.requestId === requestId &&
    state.threadId === threadId
  );
}

/**
 * Réduction pure et testable du flux de chat.
 *
 * - un delta n'est accepté que pour le `request_id` du tour courant ;
 * - un delta tardif après `done`/`error` est ignoré ;
 * - `done` remplace le texte provisoire par le texte final ;
 * - `error` vide le texte provisoire et conserve le code/message brut ;
 * - `reset` revient à l'état initial.
 */
export function chatStreamReducer(
  state: ChatStreamState,
  action: ChatStreamAction,
): ChatStreamState {
  switch (action.type) {
    case "begin":
      return {
        status: "streaming",
        requestId: action.requestId,
        threadId: action.threadId,
        text: "",
        outcome: null,
        error: null,
      };
    case "delta":
      if (!isCurrent(state, action.requestId, action.threadId)) {
        return state;
      }
      return { ...state, text: state.text + action.delta };
    case "done":
      if (!isCurrent(state, action.requestId, action.threadId)) {
        return state;
      }
      return {
        status: "done",
        requestId: state.requestId,
        threadId: state.threadId,
        text: action.outcome.text,
        outcome: action.outcome,
        error: null,
      };
    case "error":
      if (!isCurrent(state, action.requestId, action.threadId)) {
        return state;
      }
      return {
        status: "error",
        requestId: state.requestId,
        threadId: state.threadId,
        text: "",
        outcome: null,
        error: { code: action.code, message: action.message },
      };
    case "reset":
      return INITIAL_CHAT_STREAM;
    default:
      return state;
  }
}

// ------------------------------------------------------- liste des fils

/** Titre sentinelle que le noyau remplace par le premier message envoyé. */
export const DEFAULT_THREAD_TITLE = "Nouvelle conversation";

/**
 * Affiche le titre lisible d'une conversation. Le titre sentinelle reste en
 * français côté base (le noyau le reconnaît) mais est localisé à l'affichage.
 */
export function displayThreadTitle(title: string, defaultLabel: string): string {
  return title === DEFAULT_THREAD_TITLE || title.trim().length === 0
    ? defaultLabel
    : title;
}

function timestamp(value: string): number {
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? 0 : parsed;
}

/** Tri stable des conversations : plus récemment mises à jour en premier. */
export function sortThreadsByUpdated(threads: readonly Thread[]): Thread[] {
  return [...threads].sort(
    (left, right) =>
      timestamp(right.updated_at) - timestamp(left.updated_at) ||
      timestamp(right.created_at) - timestamp(left.created_at) ||
      left.id.localeCompare(right.id),
  );
}

/**
 * Ne garde que les conversations du projet actif, puis applique une recherche
 * locale sur le titre lorsqu'une requête est fournie. La recherche serveur
 * (`threads_search`) reste la source de vérité pour le contenu des messages.
 */
export function filterThreads(
  threads: readonly Thread[],
  projectId: string,
  query = "",
): Thread[] {
  const needle = query.trim().toLocaleLowerCase();
  return threads.filter(
    (thread) =>
      thread.project_id === projectId &&
      (needle.length === 0 || thread.title.toLocaleLowerCase().includes(needle)),
  );
}

// ------------------------------------------------------------ dates

const RELATIVE_STEPS: ReadonlyArray<[Intl.RelativeTimeFormatUnit, number]> = [
  ["year", 365 * 24 * 60 * 60],
  ["month", 30 * 24 * 60 * 60],
  ["week", 7 * 24 * 60 * 60],
  ["day", 24 * 60 * 60],
  ["hour", 60 * 60],
  ["minute", 60],
  ["second", 1],
];

/** Date relative réelle (« il y a 3 min », « hier »), sans texte simulé. */
export function formatRelativeTime(
  value: string | null | undefined,
  locale: string,
  now: number = Date.now(),
): string {
  if (!value) {
    return "—";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  const deltaSeconds = (date.getTime() - now) / 1000;
  const absolute = Math.abs(deltaSeconds);
  const [unit, size] =
    RELATIVE_STEPS.find(([, step]) => absolute >= step) ??
    (["second", 1] as [Intl.RelativeTimeFormatUnit, number]);
  const amount = Math.round(deltaSeconds / size);
  const tag = locale.toLowerCase().startsWith("fr") ? "fr-FR" : "en-US";
  try {
    return new Intl.RelativeTimeFormat(tag, { numeric: "auto", style: "short" }).format(
      amount,
      unit,
    );
  } catch {
    return "—";
  }
}

// ---------------------------------------------------- métadonnées message

export interface MessageMetadata {
  usage: ChatUsage | null;
  cancelled: boolean;
  finishReason: string | null;
  elapsedMs: number | null;
  provider: string | null;
  model: string | null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function tokenCount(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

/** Lit les métadonnées persistées sans jamais jeter sur une charge malformée. */
export function readMessageMetadata(metadata: unknown): MessageMetadata {
  const record = asRecord(metadata);
  if (!record) {
    return {
      usage: null,
      cancelled: false,
      finishReason: null,
      elapsedMs: null,
      provider: null,
      model: null,
    };
  }
  const usageRecord = asRecord(record.usage);
  const usage = usageRecord
    ? {
        input_tokens: tokenCount(usageRecord.input_tokens),
        output_tokens: tokenCount(usageRecord.output_tokens),
      }
    : null;
  return {
    usage,
    cancelled: record.cancelled === true,
    finishReason: typeof record.finish_reason === "string" ? record.finish_reason : null,
    elapsedMs:
      typeof record.elapsed_ms === "number" && Number.isFinite(record.elapsed_ms)
        ? record.elapsed_ms
        : null,
    provider: typeof record.provider === "string" ? record.provider : null,
    model: typeof record.model === "string" ? record.model : null,
  };
}

/** Construit l'objet `Message` final à partir d'un `ChatOutcome`. */
export function messageFromOutcome(
  outcome: ChatOutcome,
  threadId: string,
  providerId: string,
  model: string,
  createdAt: string = new Date().toISOString(),
): Message {
  return {
    id: outcome.assistant_message_id,
    thread_id: threadId,
    role: "assistant",
    content: outcome.text,
    metadata: {
      provider: providerId,
      model,
      cancelled: outcome.cancelled,
      usage: {
        input_tokens: outcome.usage.input_tokens,
        output_tokens: outcome.usage.output_tokens,
      },
      finish_reason: outcome.finish_reason,
      elapsed_ms: outcome.elapsed_ms,
    },
    created_at: createdAt,
  };
}

/** Nombre total de jetons d'un usage, ou `null` si le fournisseur ne le donne pas. */
export function totalTokens(usage: ChatUsage | null): number | null {
  if (!usage) {
    return null;
  }
  if (usage.input_tokens === null && usage.output_tokens === null) {
    return null;
  }
  return (usage.input_tokens ?? 0) + (usage.output_tokens ?? 0);
}
