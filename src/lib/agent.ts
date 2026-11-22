import type {
  AgentAdapterReport,
  AgentApprovalView,
  AgentEventRow,
  AgentSessionStarted,
  PendingDiff,
} from "./api";

// ------------------------------------------------------------------ bornes

/**
 * Nombre maximal d'événements conservés pour une session. Au-delà, les plus
 * anciens sont retirés : l'affichage reste fluide sur une session bavarde.
 */
export const MAX_AGENT_EVENTS = 1_500;

/** Texte cumulé maximal (stdout + stderr) conservé pour une session. */
export const MAX_AGENT_OUTPUT_BYTES = 256 * 1024;

/**
 * Rétention des événements persistés par le noyau (`seq < 4000`). On relit
 * cette borne pour que la troncature garde réellement les événements les plus
 * récents d'une session rechargée après redémarrage.
 */
export const MAX_PERSISTED_AGENT_EVENTS = 4_000;

/** Garde-fou sur l'ensemble des clés de déduplication (croissance mémoire). */
const MAX_SEEN_KEYS = 8_000;

// ------------------------------------------------------------------- états

/**
 * Phase affichée pour une session. Les phases du noyau (`starting`, `ready`,
 * `running`, `waiting_approval`, `completed`, `failed`, `cancelled`) sont
 * complétées par les phases propres à l'interface : `idle` (rien en cours) et
 * `approving` (demande d'approbation affichée, session pas encore lancée).
 */
export type AgentPhase =
  | "idle"
  | "approving"
  | "starting"
  | "running"
  | "waiting_approval"
  | "completed"
  | "cancelled"
  | "failed";

const TERMINAL_PHASES: ReadonlySet<AgentPhase> = new Set([
  "completed",
  "cancelled",
  "failed",
]);

const ACTIVE_PHASES: ReadonlySet<AgentPhase> = new Set([
  "approving",
  "starting",
  "running",
  "waiting_approval",
]);

/** Une session est-elle encore en cours (ou sur le point de démarrer) ? */
export function isActiveAgentPhase(phase: AgentPhase): boolean {
  return ACTIVE_PHASES.has(phase);
}

/** La session est-elle terminée (quel que soit le résultat) ? */
export function isTerminalAgentPhase(phase: AgentPhase): boolean {
  return TERMINAL_PHASES.has(phase);
}

/** Phase où une demande d'approbation ou un lancement est déjà engagé. */
export function isPendingAgentPhase(phase: AgentPhase): boolean {
  return phase === "approving" || phase === "starting";
}

/** Clé i18n du libellé d'une phase (toujours non vide dans les catalogues). */
export function agentPhaseLabelKey(phase: AgentPhase): string {
  return `agent_mode.phase_${phase}`;
}

/** Traduit un `AgentState` du noyau vers la phase d'affichage. */
export function mapAgentState(state: string | null | undefined): AgentPhase | null {
  switch (state) {
    case "starting":
    case "ready":
      return "starting";
    case "running":
      return "running";
    case "waiting_approval":
      return "waiting_approval";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "cancelled":
      return "cancelled";
    default:
      return null;
  }
}

/**
 * Phase effective : un état terminal (événement temps réel ou session
 * persistée) prime toujours sur une phase de lancement locale. C'est ce qui
 * permet d'afficher une session `running` rechargée après redémarrage sans la
 * présenter comme terminée, et l'inverse.
 */
export function resolveAgentPhase(
  derived: AgentPhase | null,
  sessionState: string | null | undefined,
  launchPhase: AgentPhase,
): AgentPhase {
  if (derived !== null && isTerminalAgentPhase(derived)) {
    return derived;
  }
  const mapped = mapAgentState(sessionState);
  if (mapped !== null && isTerminalAgentPhase(mapped)) {
    return mapped;
  }
  if (derived !== null) {
    return derived;
  }
  if (mapped !== null) {
    return mapped;
  }
  return launchPhase;
}

// --------------------------------------------------------------- événements

/** Événement affichable, qu'il vienne de l'historique ou du flux temps réel. */
export interface AgentEvent {
  seq: number;
  kind: string;
  payload: unknown;
  created_at: string | null;
}

/** Charge utile de `ih://agent-event` (les événements `started`/fin ont `seq = -1`). */
export interface AgentLiveEvent {
  session_id: string;
  seq: number;
  kind: string;
  payload: unknown;
}

export type AgentOutputStream = "stdout" | "stderr";

export function toAgentEvent(row: AgentEventRow): AgentEvent {
  return {
    seq: row.seq,
    kind: row.kind,
    payload: row.payload,
    created_at: row.created_at,
  };
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * Clé de déduplication. Les événements persistés (`seq >= 0`) sont uniques par
 * `seq` : c'est cette clé qui fusionne l'historique `agent_session_events` et
 * le temps réel `ih://agent-event`. Les événements de début et de fin ne sont
 * pas persistés et partagent `seq = -1` : leur `kind` fait partie de la clé.
 */
export function agentEventKey(event: Pick<AgentEvent, "seq" | "kind">): string {
  return event.seq >= 0 ? `s${event.seq}` : `l${event.seq}:${event.kind}`;
}

/** Lit `{stream, data}` sans jamais jeter sur une charge malformée. */
export function readOutputPayload(
  payload: unknown,
): { stream: AgentOutputStream; data: string } | null {
  const record = asRecord(payload);
  if (!record || typeof record.data !== "string") {
    return null;
  }
  return {
    stream: record.stream === "stderr" ? "stderr" : "stdout",
    data: record.data,
  };
}

/** Lit `{state}` d'un événement `state`. */
export function readStatePayload(payload: unknown): AgentPhase | null {
  const record = asRecord(payload);
  return record ? mapAgentState(typeof record.state === "string" ? record.state : null) : null;
}

export interface AgentTerminalPayload {
  summary: string | null;
  files: string[];
  changesPrepared: number | null;
  error: string | null;
}

/** Lit la charge d'un événement terminal (`completed`, `cancelled`, `error`). */
export function readTerminalPayload(payload: unknown): AgentTerminalPayload {
  const record = asRecord(payload);
  if (!record) {
    return { summary: null, files: [], changesPrepared: null, error: null };
  }
  const files = Array.isArray(record.files)
    ? record.files.filter((entry): entry is string => typeof entry === "string")
    : [];
  return {
    summary: typeof record.summary === "string" ? record.summary : null,
    files,
    changesPrepared:
      typeof record.changes_prepared === "number" && Number.isFinite(record.changes_prepared)
        ? record.changes_prepared
        : null,
    error: typeof record.error === "string" ? record.error : null,
  };
}

function utf8Length(value: string): number {
  if (typeof TextEncoder === "undefined") {
    return value.length;
  }
  return new TextEncoder().encode(value).length;
}

/** Octets de texte produits par un événement (`0` hors sortie). */
export function agentEventOutputBytes(event: AgentEvent): number {
  if (event.kind !== "output") {
    return 0;
  }
  const output = readOutputPayload(event.payload);
  return output ? utf8Length(output.data) : 0;
}

// ------------------------------------------------------------ état du flux

export interface AgentEventStream {
  /** Session suivie ; `null` quand aucune session n'est sélectionnée. */
  sessionId: string | null;
  /** Événements triés par `seq`, sans doublon, bornés. */
  events: AgentEvent[];
  /** Octets de sortie conservés (borne mémoire). */
  outputBytes: number;
  /** Phase dérivée des kinds, conservée même quand la liste est tronquée. */
  phase: AgentPhase | null;
  summary: string | null;
  files: string[];
  changesPrepared: number | null;
  error: string | null;
  /** Vrai dès qu'un événement a été retiré pour respecter une borne. */
  bounded: boolean;
  /** Clés déjà vues (déduplication temps réel + historique). */
  seen: ReadonlySet<string>;
}

export const INITIAL_AGENT_EVENTS: AgentEventStream = {
  sessionId: null,
  events: [],
  outputBytes: 0,
  phase: null,
  summary: null,
  files: [],
  changesPrepared: null,
  error: null,
  bounded: false,
  seen: new Set<string>(),
};

export type AgentEventAction =
  | { type: "select"; sessionId: string | null }
  | { type: "history"; sessionId: string; events: readonly AgentEventRow[] }
  | { type: "live"; event: AgentLiveEvent }
  | { type: "reset" };

interface AgentEventPatch {
  phase?: AgentPhase;
  summary?: string | null;
  files?: string[];
  changesPrepared?: number | null;
  error?: string | null;
}

function nextPhase(current: AgentPhase | null, candidate: AgentPhase): AgentPhase {
  if (current !== null && isTerminalAgentPhase(current) && !isTerminalAgentPhase(candidate)) {
    // Un état tardif ne doit jamais rouvrir une session terminée.
    return current;
  }
  return candidate;
}

function deriveEventPatch(state: AgentEventStream, event: AgentEvent): AgentEventPatch {
  switch (event.kind) {
    case "started":
      return { phase: nextPhase(state.phase, "running") };
    case "state": {
      const mapped = readStatePayload(event.payload);
      return mapped === null ? {} : { phase: nextPhase(state.phase, mapped) };
    }
    case "completed":
    case "cancelled":
    case "error": {
      const terminal = readTerminalPayload(event.payload);
      const phase: AgentPhase =
        event.kind === "completed" ? "completed" : event.kind === "cancelled" ? "cancelled" : "failed";
      return {
        phase: nextPhase(state.phase, phase),
        summary: terminal.summary,
        files: terminal.files,
        changesPrepared: terminal.changesPrepared,
        error: terminal.error,
      };
    }
    default:
      return {};
  }
}

/**
 * Insère un événement à sa place dans l'ordre des `seq`. À `seq` égal (début et
 * fin non persistés), l'ordre d'arrivée est conservé.
 */
export function insertAgentEvent(
  events: readonly AgentEvent[],
  event: AgentEvent,
): AgentEvent[] {
  let index = events.length;
  while (index > 0 && events[index - 1].seq > event.seq) {
    index -= 1;
  }
  return [...events.slice(0, index), event, ...events.slice(index)];
}

function boundEvents(
  events: AgentEvent[],
  outputBytes: number,
): { events: AgentEvent[]; outputBytes: number } {
  let start = 0;
  let bytes = outputBytes;
  while (
    start < events.length &&
    (events.length - start > MAX_AGENT_EVENTS || bytes > MAX_AGENT_OUTPUT_BYTES)
  ) {
    bytes -= agentEventOutputBytes(events[start]);
    start += 1;
  }
  return {
    events: start === 0 ? events : events.slice(start),
    outputBytes: Math.max(bytes, 0),
  };
}

function pruneSeen(seen: Set<string>, events: readonly AgentEvent[]): ReadonlySet<string> {
  if (seen.size <= MAX_SEEN_KEYS) {
    return seen;
  }
  // Reconstruction bornée : après des milliers d'événements, seules les clés
  // des événements encore affichés sont conservées.
  const rebuilt = new Set<string>();
  for (const event of events) {
    rebuilt.add(agentEventKey(event));
  }
  return rebuilt;
}

/** Ajoute un événement (ordre, déduplication, bornes) sans effet de bord. */
export function appendAgentEvent(
  state: AgentEventStream,
  event: AgentEvent,
): AgentEventStream {
  const key = agentEventKey(event);
  if (state.seen.has(key)) {
    return state;
  }
  const seen = new Set(state.seen);
  seen.add(key);
  const events = insertAgentEvent(state.events, event);
  const bounded = boundEvents(events, state.outputBytes + agentEventOutputBytes(event));
  const patch = deriveEventPatch(state, event);
  return {
    ...state,
    events: bounded.events,
    outputBytes: bounded.outputBytes,
    bounded: state.bounded || bounded.events.length !== events.length,
    seen: pruneSeen(seen, bounded.events),
    ...patch,
  };
}

/**
 * Réduction pure du flux d'événements d'une session.
 *
 * - les événements d'une autre session sont ignorés ;
 * - l'historique et le temps réel sont fusionnés sans doublon (clé `seq`) ;
 * - la liste est ordonnée par `seq` et bornée (1 500 événements, 256 Kio de
 *   sortie), les plus récents étant conservés ;
 * - la phase dérivée et le résumé terminal survivent à la troncature.
 */
export function agentEventReducer(
  state: AgentEventStream,
  action: AgentEventAction,
): AgentEventStream {
  switch (action.type) {
    case "select":
      return {
        ...INITIAL_AGENT_EVENTS,
        sessionId: action.sessionId,
        seen: new Set<string>(),
      };
    case "reset":
      return INITIAL_AGENT_EVENTS;
    case "history": {
      if (state.sessionId === null || state.sessionId !== action.sessionId) {
        return state;
      }
      let next = state;
      for (const row of action.events) {
        if (row.session_id !== action.sessionId) {
          continue;
        }
        next = appendAgentEvent(next, toAgentEvent(row));
      }
      return next;
    }
    case "live": {
      if (state.sessionId === null || action.event.session_id !== state.sessionId) {
        return state;
      }
      return appendAgentEvent(state, {
        seq: action.event.seq,
        kind: action.event.kind,
        payload: action.event.payload,
        created_at: null,
      });
    }
    default:
      return state;
  }
}

/** Libellé i18n d'un kind connu, ou `null` pour un kind non répertorié. */
export function agentEventLabelKey(kind: string): string | null {
  switch (kind) {
    case "started":
      return "agent_mode.kind_started";
    case "output":
      return "agent_mode.kind_output";
    case "state":
      return "agent_mode.kind_state";
    case "completed":
      return "agent_mode.kind_completed";
    case "cancelled":
      return "agent_mode.kind_cancelled";
    case "error":
      return "agent_mode.kind_error";
    default:
      return null;
  }
}

// --------------------------------------------------- flux d'approbation

/**
 * Machine à états du lancement. Elle suit exactement l'enchaînement réel :
 * demande d'approbation → approbation (ou refus) → démarrage → session en
 * cours. Refuser ou annuler revient à l'état initial sans session.
 */
export interface AgentLaunchState {
  phase: AgentPhase;
  approval: AgentApprovalView | null;
  session: AgentSessionStarted | null;
  agentId: string;
  task: string;
  /** Vrai après un refus explicite (aucune session lancée). */
  refused: boolean;
}

export const INITIAL_AGENT_LAUNCH: AgentLaunchState = {
  phase: "idle",
  approval: null,
  session: null,
  agentId: "",
  task: "",
  refused: false,
};

export type AgentLaunchAction =
  | { type: "request"; approval: AgentApprovalView; agentId: string; task: string }
  | { type: "dismiss" }
  | { type: "refuse" }
  | { type: "approve" }
  | { type: "started"; session: AgentSessionStarted }
  | { type: "reset" };

/** Réduction pure des transitions d'approbation (aucun appel IPC ici). */
export function agentLaunchReducer(
  state: AgentLaunchState,
  action: AgentLaunchAction,
): AgentLaunchState {
  switch (action.type) {
    case "request":
      if (isPendingAgentPhase(state.phase)) {
        return state;
      }
      return {
        phase: "approving",
        approval: action.approval,
        session: null,
        agentId: action.agentId,
        task: action.task,
        refused: false,
      };
    case "dismiss":
      if (state.phase !== "approving") {
        return state;
      }
      return { ...state, phase: "idle", approval: null };
    case "refuse":
      if (state.phase !== "approving") {
        return state;
      }
      return {
        ...state,
        phase: "idle",
        approval: null,
        session: null,
        refused: true,
      };
    case "approve":
      if (state.phase !== "approving") {
        return state;
      }
      return { ...state, phase: "starting" };
    case "started":
      if (state.phase !== "starting") {
        return state;
      }
      return {
        ...state,
        phase: "running",
        session: action.session,
        approval: null,
        refused: false,
      };
    case "reset":
      return INITIAL_AGENT_LAUNCH;
    default:
      return state;
  }
}

// ---------------------------------------------------- sélection adaptateur

/** Un adaptateur n'est sélectionnable que s'il a réellement été détecté. */
export function isAdapterSelectable(adapter: AgentAdapterReport): boolean {
  return adapter.detected;
}

/** Adaptateurs réellement lançables. */
export function selectableAdapters(
  adapters: readonly AgentAdapterReport[],
): AgentAdapterReport[] {
  return adapters.filter(isAdapterSelectable);
}

/** Premier adaptateur détecté, ou `null` si aucun ne l'est. */
export function pickDefaultAdapter(
  adapters: readonly AgentAdapterReport[],
): AgentAdapterReport | null {
  return adapters.find(isAdapterSelectable) ?? null;
}

/**
 * Sélection effective : l'adaptateur courant s'il est détecté, sinon le
 * premier détecté, sinon `null` (aucun lancement possible).
 */
export function resolveAdapterSelection(
  adapters: readonly AgentAdapterReport[],
  currentId: string,
): AgentAdapterReport | null {
  const current = adapters.find((adapter) => adapter.id === currentId);
  if (current && isAdapterSelectable(current)) {
    return current;
  }
  return pickDefaultAdapter(adapters);
}

export type AgentLaunchBlocked = "adapter" | "task" | "active" | "busy" | null;

/** Raison réelle du blocage du bouton « Lancer » (`null` = lançable). */
export function agentLaunchBlockedReason(params: {
  adapter: AgentAdapterReport | null;
  task: string;
  active: boolean;
  busy: boolean;
}): AgentLaunchBlocked {
  if (params.busy) {
    return "busy";
  }
  if (!params.adapter || !isAdapterSelectable(params.adapter)) {
    return "adapter";
  }
  if (params.task.trim().length === 0) {
    return "task";
  }
  if (params.active) {
    return "active";
  }
  return null;
}

/** Le bouton « Lancer » est-il actionnable ? */
export function canLaunchAgent(params: {
  adapter: AgentAdapterReport | null;
  task: string;
  active: boolean;
  busy: boolean;
}): boolean {
  return agentLaunchBlockedReason(params) === null;
}

// ------------------------------------------------------- diffs d'une session

export const AGENT_DIFF_ORIGIN_PREFIX = "agent:";

/** Origine exacte inscrite par le noyau dans `diff_pending`. */
export function agentDiffOrigin(sessionId: string): string {
  return `${AGENT_DIFF_ORIGIN_PREFIX}${sessionId}`;
}

/** Modifications en attente produites par une session donnée. */
export function filterAgentPendingDiffs(
  diffs: readonly PendingDiff[],
  sessionId: string | null,
): PendingDiff[] {
  if (!sessionId) {
    return [];
  }
  const origin = agentDiffOrigin(sessionId);
  return diffs.filter((entry) => entry.origin === origin);
}

/** Nombre de lignes de sortie réellement affichées (utile aux compteurs). */
export function agentOutputEventCount(events: readonly AgentEvent[]): number {
  return events.reduce((count, event) => count + (event.kind === "output" ? 1 : 0), 0);
}
