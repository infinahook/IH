import type { PendingDiff, TeamRunRequest, TeamRunResult } from "./api";

// ------------------------------------------------------------- constantes

/** Origine exacte inscrite par le noyau dans `diff_pending` pour une course. */
export const TEAM_DIFF_ORIGIN_PREFIX = "team:";

export const MIN_TEAM_WORKERS = 1;
export const MAX_TEAM_WORKERS = 3;
export const MIN_TEAM_WORK_ITEMS = 1;
export const MAX_TEAM_WORK_ITEMS = 4;

// ------------------------------------------------------------------ rôles

export type TeamRole = "lead" | "worker" | "reviewer";

export const TEAM_ROLES: readonly TeamRole[] = ["lead", "worker", "reviewer"];

/** Normalise le rôle renvoyé par le noyau ; `null` pour une valeur inconnue. */
export function normalizeTeamRole(value: string | null | undefined): TeamRole | null {
  switch ((value ?? "").trim().toLowerCase()) {
    case "lead":
      return "lead";
    case "worker":
      return "worker";
    case "reviewer":
      return "reviewer";
    default:
      return null;
  }
}

/** Clé i18n du libellé d'un rôle normalisé. */
export function teamRoleLabelKey(role: TeamRole): string {
  return `team.role_${role}`;
}

// -------------------------------------------------------- construction

/** État du formulaire d'équipe, indépendant de React. */
export interface TeamForm {
  projectId: string;
  goal: string;
  lead: string;
  workers: readonly string[];
  reviewer: string | null;
  workItems: number;
}

export type TeamFieldError =
  | "project"
  | "goal"
  | "lead"
  | "workers"
  | "work_items";

export interface TeamRequestBuild {
  request: TeamRunRequest | null;
  error: TeamFieldError | null;
}

function invalid(error: TeamFieldError): TeamRequestBuild {
  return { request: null, error };
}

/**
 * Construit la requête IPC exacte (`snake_case` attendu par le noyau).
 *
 * Validation : objectif non vide, lead obligatoire, 1 à 3 workers distincts,
 * relecteur optionnel mais jamais confondu avec le lead ou un worker (le noyau
 * refuse un agent présent deux fois dans l'équipe).
 */
export function buildTeamRequest(form: TeamForm): TeamRequestBuild {
  const projectId = form.projectId.trim();
  if (projectId.length === 0) {
    return invalid("project");
  }
  const goal = form.goal.trim();
  if (goal.length === 0) {
    return invalid("goal");
  }
  const lead = form.lead.trim();
  if (lead.length === 0) {
    return invalid("lead");
  }
  const seen = new Set<string>();
  const workers: string[] = [];
  for (const raw of form.workers) {
    const worker = raw.trim();
    if (worker.length === 0) {
      continue;
    }
    if (seen.has(worker)) {
      continue; // un même agent peut tenir plusieurs rôles
    }
    seen.add(worker);
    workers.push(worker);
  }
  if (workers.length < MIN_TEAM_WORKERS || workers.length > MAX_TEAM_WORKERS) {
    return invalid("workers");
  }
  const reviewer = (form.reviewer ?? "").trim();
  const workItems = Math.trunc(Number(form.workItems));
  if (!Number.isFinite(workItems) || workItems < MIN_TEAM_WORK_ITEMS || workItems > MAX_TEAM_WORK_ITEMS) {
    return invalid("work_items");
  }
  return {
    request: {
      project_id: projectId,
      goal,
      lead,
      workers,
      reviewer: reviewer.length > 0 ? reviewer : null,
      work_items: workItems,
    },
    error: null,
  };
}

/** Clé i18n du blocage du bouton « Lancer l'équipe » (`null` = lançable). */
export function teamFieldErrorKey(error: TeamFieldError | null): string | null {
  return error === null ? null : `team.error.${error}`;
}

// --------------------------------------------------------------- événements

/**
 * Événements de `ih://team-event`, normalisés.
 *
 * - `task` : démarrage (ou mise à jour) d'une tâche ;
 * - `terminal` : fin de la course, avec résumé et fichiers.
 */
export interface TeamTaskEvent {
  kind: "task";
  runId: string;
  taskId: string;
  title: string;
  role: TeamRole | null;
  rawRole: string;
  state: string;
}

export interface TeamTerminalEvent {
  kind: "terminal";
  runId: string;
  state: "completed" | "failed";
  summary: string | null;
  files: string[];
}

export type TeamEvent = TeamTaskEvent | TeamTerminalEvent;

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function stringList(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is string => typeof entry === "string")
    : [];
}

/** Lit une charge `ih://team-event` sans jamais jeter sur une charge malformée. */
export function readTeamEvent(payload: unknown): TeamEvent | null {
  const record = asRecord(payload);
  if (!record) {
    return null;
  }
  const runId = typeof record.run_id === "string" ? record.run_id.trim() : "";
  if (runId.length === 0) {
    return null;
  }
  const taskId = typeof record.task_id === "string" ? record.task_id : "";
  if (taskId.length > 0) {
    const rawRole = typeof record.role === "string" ? record.role : "";
    return {
      kind: "task",
      runId,
      taskId,
      title: typeof record.title === "string" && record.title.length > 0 ? record.title : taskId,
      role: normalizeTeamRole(rawRole),
      rawRole,
      state: typeof record.state === "string" && record.state.length > 0 ? record.state : "running",
    };
  }
  if (record.state !== "completed" && record.state !== "failed") {
    return null;
  }
  return {
    kind: "terminal",
    runId,
    state: record.state,
    summary: typeof record.summary === "string" ? record.summary : null,
    files: stringList(record.files),
  };
}

// ------------------------------------------------------------ état de course

export type TeamRunStatus = "idle" | "running" | "completed" | "failed";

export interface TeamTaskView {
  taskId: string;
  title: string;
  role: TeamRole | null;
  rawRole: string;
  state: string;
  sessions: number;
}

export interface TeamRunState {
  /** Course suivie ; adoptée au premier événement, confirmée par `team_run`. */
  runId: string | null;
  status: TeamRunStatus;
  tasks: TeamTaskView[];
  summary: string | null;
  files: string[];
}

export const INITIAL_TEAM_RUN: TeamRunState = {
  runId: null,
  status: "idle",
  tasks: [],
  summary: null,
  files: [],
};

function taskFromEvent(event: TeamTaskEvent): TeamTaskView {
  return {
    taskId: event.taskId,
    title: event.title,
    role: event.role,
    rawRole: event.rawRole,
    state: event.state,
    sessions: 0,
  };
}

function taskFromResult(task: TeamRunResult["tasks"][number]): TeamTaskView {
  const role = normalizeTeamRole(task.role);
  return {
    taskId: task.task_id,
    title: task.title,
    role,
    rawRole: typeof task.role === "string" ? task.role : "",
    state: task.state,
    sessions: Number.isFinite(task.sessions) ? task.sessions : 0,
  };
}

/**
 * Insère ou met à jour une tâche **par `task_id`** : le dédoublonnage conserve
 * la position d'origine et cumule le nombre de sessions observé.
 */
export function upsertTeamTask(
  tasks: readonly TeamTaskView[],
  incoming: TeamTaskView,
): TeamTaskView[] {
  const index = tasks.findIndex((task) => task.taskId === incoming.taskId);
  if (index === -1) {
    return [...tasks, incoming];
  }
  const next = tasks.slice();
  const current = tasks[index];
  next[index] = {
    ...current,
    ...incoming,
    sessions: Math.max(current.sessions, incoming.sessions),
  };
  return next;
}

export type TeamRunAction =
  | { type: "start" }
  | { type: "event"; event: TeamEvent }
  | { type: "result"; result: TeamRunResult }
  | { type: "fail" }
  | { type: "reset" };

function isTerminalStatus(status: TeamRunStatus): boolean {
  return status === "completed" || status === "failed";
}

/**
 * Réduction pure d'une course d'équipe.
 *
 * - la course suivie est fixée au premier événement puis verrouillée : un
 *   événement d'une autre course est ignoré ;
 * - un état terminal est définitif (un événement tardif ne le rouvre pas) ;
 * - les tâches sont dédoublonnées par `task_id`, la première position gagne ;
 * - le résultat final de `team_run` fusionne les tâches déjà vues sans doublon.
 */
export function teamRunReducer(state: TeamRunState, action: TeamRunAction): TeamRunState {
  switch (action.type) {
    case "reset":
      return INITIAL_TEAM_RUN;
    case "start":
      return { ...INITIAL_TEAM_RUN, status: "running" };
    case "fail":
      return state.status === "running" ? { ...state, status: "failed" } : state;
    case "event": {
      const event = action.event;
      if (state.runId !== null && state.runId !== event.runId) {
        return state;
      }
      if (isTerminalStatus(state.status)) {
        return state;
      }
      if (event.kind === "task") {
        return {
          ...state,
          runId: state.runId ?? event.runId,
          status: "running",
          tasks: upsertTeamTask(state.tasks, taskFromEvent(event)),
        };
      }
      return {
        ...state,
        runId: state.runId ?? event.runId,
        status: event.state,
        summary: event.summary ?? state.summary,
        files: event.files,
      };
    }
    case "result": {
      const result = action.result;
      if (state.runId !== null && result.run_id !== state.runId) {
        return state;
      }
      let tasks = state.tasks;
      for (const task of result.tasks) {
        tasks = upsertTeamTask(tasks, taskFromResult(task));
      }
      const status = isTerminalStatus(state.status)
        ? state.status
        : tasks.some((task) => task.state === "failed" || task.state === "cancelled")
          ? "failed"
          : "completed";
      return {
        ...state,
        runId: result.run_id,
        status,
        tasks,
        summary: result.summary,
        files: result.files,
      };
    }
    default:
      return state;
  }
}

/** Libellé i18n d'un état de tâche connu (`null` : afficher l'état brut). */
export function teamTaskStateLabelKey(state: string): string | null {
  switch (state) {
    case "pending":
    case "running":
    case "completed":
    case "failed":
    case "cancelled":
      return `team.task_state_${state}`;
    default:
      return null;
  }
}

/** Libellé i18n de l'état global de la course. */
export function teamStatusLabelKey(status: TeamRunStatus): string {
  return `team.status_${status}`;
}

// ------------------------------------------------------ diffs de la course

/** Origine exacte inscrite par le noyau pour une course donnée. */
export function teamDiffOrigin(runId: string): string {
  return `${TEAM_DIFF_ORIGIN_PREFIX}${runId}`;
}

/** Modifications en attente produites par une course d'équipe. */
export function filterTeamPendingDiffs(
  diffs: readonly PendingDiff[],
  runId: string | null,
): PendingDiff[] {
  if (!runId) {
    return [];
  }
  const origin = teamDiffOrigin(runId);
  return diffs.filter((entry) => entry.origin === origin);
}
