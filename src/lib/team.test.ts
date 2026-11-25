import { describe, expect, it } from "vitest";
import type { PendingDiff, TeamRunResult } from "./api";
import {
  INITIAL_TEAM_RUN,
  TEAM_DIFF_ORIGIN_PREFIX,
  buildTeamRequest,
  filterTeamPendingDiffs,
  normalizeTeamRole,
  readTeamEvent,
  teamDiffOrigin,
  teamFieldErrorKey,
  teamRoleLabelKey,
  teamRunReducer,
  teamStatusLabelKey,
  teamTaskStateLabelKey,
  upsertTeamTask,
  type TeamEvent,
  type TeamForm,
  type TeamRunState,
  type TeamTaskView,
  teamRequiresUnsandboxedAck,
} from "./team";

// ---------------------------------------------------------------- fabriques

function form(overrides: Partial<TeamForm> = {}): TeamForm {
  return {
    projectId: "projet-1",
    goal: "  Ajouter la validation TOML  ",
    lead: "claude",
    workers: ["codex"],
    reviewer: null,
    workItems: 2,
    ...overrides,
  };
}

function taskEvent(
  overrides: Partial<Extract<TeamEvent, { kind: "task" }>> = {},
): Extract<TeamEvent, { kind: "task" }> {
  return {
    kind: "task",
    runId: "run-1",
    taskId: "t1",
    title: "Analyser l'objectif",
    role: "lead",
    rawRole: "lead",
    state: "running",
    ...overrides,
  };
}

function terminalEvent(
  overrides: Partial<Extract<TeamEvent, { kind: "terminal" }>> = {},
): Extract<TeamEvent, { kind: "terminal" }> {
  return {
    kind: "terminal",
    runId: "run-1",
    state: "completed",
    summary: "course terminée — 1 fichier(s) en attente",
    files: ["src/lib.rs"],
    ...overrides,
  };
}

function runningState(): TeamRunState {
  return teamRunReducer(INITIAL_TEAM_RUN, { type: "start" });
}

function taskView(overrides: Partial<TeamTaskView> = {}): TeamTaskView {
  return {
    taskId: "t1",
    title: "Analyser l'objectif",
    role: "lead",
    rawRole: "lead",
    state: "running",
    sessions: 0,
    ...overrides,
  };
}

function result(overrides: Partial<TeamRunResult> = {}): TeamRunResult {
  return {
    run_id: "run-1",
    summary: "course terminée",
    files: ["src/lib.rs"],
    changes_prepared: 1,
    tasks: [
      {
        task_id: "t1",
        title: "Analyser l'objectif",
        role: "lead",
        state: "completed",
        sessions: 1,
      },
    ],
    ...overrides,
  };
}

function diff(id: string, origin: string): PendingDiff {
  return {
    id,
    project_id: "projet-1",
    path: `src/${id}.rs`,
    origin,
    delete: false,
    created_at: "2026-01-01T00:00:00Z",
  };
}

// ------------------------------------------------------------ construction

describe("construction de la requête d'équipe", () => {
  it("construit la requête IPC en snake_case, bornes comprises", () => {
    const built = buildTeamRequest(form());
    expect(built.error).toBeNull();
    expect(built.request).toEqual({
      project_id: "projet-1",
      goal: "Ajouter la validation TOML",
      lead: "claude",
      workers: ["codex"],
      reviewer: null,
      work_items: 2,
      unsandboxed_ack: false,
    });
  });

  it("nettoie les identifiants et accepte un relecteur explicite", () => {
    const built = buildTeamRequest(
      form({
        lead: " claude ",
        workers: [" codex ", " gemini "],
        reviewer: " reviewer ",
        workItems: 4,
      }),
    );
    expect(built.request).toMatchObject({
      lead: "claude",
      workers: ["codex", "gemini"],
      reviewer: "reviewer",
      work_items: 4,
    });
  });

  it("refuse un objectif vide, un lead absent ou un projet absent", () => {
    expect(buildTeamRequest(form({ goal: "   " })).error).toBe("goal");
    expect(buildTeamRequest(form({ lead: "" })).error).toBe("lead");
    expect(buildTeamRequest(form({ projectId: "" })).error).toBe("project");
  });

  it("exige entre 1 et 3 workers", () => {
    expect(buildTeamRequest(form({ workers: [] })).error).toBe("workers");
    expect(buildTeamRequest(form({ workers: ["a", "b", "c", "d"] })).error).toBe("workers");
    expect(buildTeamRequest(form({ workers: ["a", "b", "c"] })).error).toBeNull();
  });

  it("accepte le même agent dans plusieurs rôles (un seul CLI installé)", () => {
    // Cas réel : OpenCode est le seul CLI détecté, il doit pouvoir être
    // lead, worker et reviewer — chaque rôle lance sa propre session.
    const solo = buildTeamRequest(
      form({ lead: "opencode", workers: ["opencode"], reviewer: "opencode" }),
    );
    expect(solo.error).toBeNull();
    expect(solo.request?.lead).toBe("opencode");
    expect(solo.request?.workers).toEqual(["opencode"]);
    expect(solo.request?.reviewer).toBe("opencode");
    // Un worker listé deux fois est dédoublonné, pas rejeté.
    const duplicate = buildTeamRequest(form({ workers: ["codex", "codex"] }));
    expect(duplicate.error).toBeNull();
    expect(duplicate.request?.workers).toEqual(["codex"]);
  });

  it("borne le nombre de tâches d'exécution entre 1 et 4", () => {
    expect(buildTeamRequest(form({ workItems: 0 })).error).toBe("work_items");
    expect(buildTeamRequest(form({ workItems: 5 })).error).toBe("work_items");
    expect(buildTeamRequest(form({ workItems: 1 })).request?.work_items).toBe(1);
  });

  it("expose la clé i18n du blocage", () => {
    expect(teamFieldErrorKey(null)).toBeNull();
    expect(teamFieldErrorKey("workers")).toBe("team.error.workers");
  });
});

// ------------------------------------------------------------- rôles

describe("acquittement hors sandbox en équipe", () => {
  const securityOf = (id: string) => ({ iso: "isolated", net: "isolated_network", none: "none" })[id];

  it("exige l'acquittement dès qu'un membre est au niveau none", () => {
    expect(teamRequiresUnsandboxedAck(["iso", "net"], securityOf)).toBe(false);
    expect(teamRequiresUnsandboxedAck(["iso", "none"], securityOf)).toBe(true);
    expect(teamRequiresUnsandboxedAck([null, "none", undefined], securityOf)).toBe(true);
    // Un agent inconnu ne déclenche pas l'exigence (le noyau refusera l'agent).
    expect(teamRequiresUnsandboxedAck(["inconnu"], securityOf)).toBe(false);
  });

  it("transmet l'acquittement dans la requête IPC", () => {
    const base = {
      projectId: "projet-1",
      goal: "Objectif",
      lead: "iso",
      workers: ["net"],
      reviewer: null,
      workItems: 2,
    };
    const withoutAck = buildTeamRequest({ ...base, unsandboxedAck: false });
    expect(withoutAck.request?.unsandboxed_ack).toBe(false);
    const withAck = buildTeamRequest({ ...base, unsandboxedAck: true });
    expect(withAck.request?.unsandboxed_ack).toBe(true);
  });
});

describe("mapping des rôles", () => {
  it("normalise les trois rôles et rejette l'inconnu", () => {
    expect(normalizeTeamRole("lead")).toBe("lead");
    expect(normalizeTeamRole("Worker")).toBe("worker");
    expect(normalizeTeamRole(" reviewer ")).toBe("reviewer");
    expect(normalizeTeamRole("observer")).toBeNull();
    expect(normalizeTeamRole(null)).toBeNull();
  });

  it("fournit les clés i18n des rôles, états et statuts", () => {
    expect(teamRoleLabelKey("lead")).toBe("team.role_lead");
    expect(teamRoleLabelKey("reviewer")).toBe("team.role_reviewer");
    expect(teamTaskStateLabelKey("running")).toBe("team.task_state_running");
    expect(teamTaskStateLabelKey("ready")).toBeNull();
    expect(teamStatusLabelKey("completed")).toBe("team.status_completed");
  });
});

// ---------------------------------------------------------- événements

describe("lecture des événements d'équipe", () => {
  it("lit un démarrage de tâche et une fin de course", () => {
    const started = readTeamEvent({
      run_id: "run-1",
      task_id: "t1",
      title: "Analyser",
      role: "lead",
      state: "running",
    });
    expect(started).toEqual({
      kind: "task",
      runId: "run-1",
      taskId: "t1",
      title: "Analyser",
      role: "lead",
      rawRole: "lead",
      state: "running",
    });

    const ended = readTeamEvent({
      run_id: "run-1",
      state: "completed",
      summary: "fini",
      files: ["a.rs", 42],
    });
    expect(ended).toEqual({
      kind: "terminal",
      runId: "run-1",
      state: "completed",
      summary: "fini",
      files: ["a.rs"],
    });
  });

  it("ignore une charge malformée", () => {
    expect(readTeamEvent(null)).toBeNull();
    expect(readTeamEvent({})).toBeNull();
    expect(readTeamEvent({ run_id: "run-1", state: "inconnu" })).toBeNull();
  });
});

// --------------------------------------------------------------- réduction

describe("réduction d'une course d'équipe", () => {
  it("passe de running à completed à la fin de la course", () => {
    let state = runningState();
    state = teamRunReducer(state, { type: "event", event: taskEvent() });
    expect(state.status).toBe("running");
    expect(state.runId).toBe("run-1");
    expect(state.tasks).toHaveLength(1);

    state = teamRunReducer(state, { type: "event", event: terminalEvent() });
    expect(state.status).toBe("completed");
    expect(state.summary).toBe("course terminée — 1 fichier(s) en attente");
    expect(state.files).toEqual(["src/lib.rs"]);
  });

  it("dédoublonne les tâches par task_id et met à jour leur état", () => {
    let state = runningState();
    state = teamRunReducer(state, { type: "event", event: taskEvent() });
    state = teamRunReducer(state, {
      type: "event",
      event: taskEvent({ state: "completed" }),
    });
    expect(state.tasks).toHaveLength(1);
    expect(state.tasks[0].state).toBe("completed");
  });

  it("conserve l'ordre de première apparition", () => {
    let state = runningState();
    state = teamRunReducer(state, {
      type: "event",
      event: taskEvent({ taskId: "t2", title: "Deuxième" }),
    });
    state = teamRunReducer(state, {
      type: "event",
      event: taskEvent({ taskId: "t1", title: "Première" }),
    });
    state = teamRunReducer(state, { type: "event", event: taskEvent({ taskId: "t2" }) });
    expect(state.tasks.map((task) => task.taskId)).toEqual(["t2", "t1"]);
  });

  it("ignore un événement d'une autre course puis n'accepte plus d'état tardif", () => {
    let state = runningState();
    state = teamRunReducer(state, { type: "event", event: taskEvent() });
    const other = teamRunReducer(state, {
      type: "event",
      event: taskEvent({ runId: "run-2", taskId: "t9" }),
    });
    expect(other).toBe(state);

    state = teamRunReducer(state, { type: "event", event: terminalEvent() });
    const late = teamRunReducer(state, {
      type: "event",
      event: taskEvent({ state: "running" }),
    });
    expect(late).toBe(state);
  });

  it("fusionne le résultat final sans dupliquer les tâches déjà reçues", () => {
    let state = runningState();
    state = teamRunReducer(state, { type: "event", event: taskEvent() });
    state = teamRunReducer(state, { type: "result", result: result() });
    expect(state.tasks).toHaveLength(1);
    expect(state.tasks[0]).toMatchObject({ state: "completed", sessions: 1 });
    expect(state.status).toBe("completed");
  });

  it("déduit un échec du résultat quand aucune tâche n'a réussi", () => {
    let state = runningState();
    state = teamRunReducer(state, {
      type: "result",
      result: result({
        summary: "course interrompue",
        tasks: [
          {
            task_id: "t1",
            title: "Analyser",
            role: "lead",
            state: "failed",
            sessions: 0,
          },
        ],
      }),
    });
    expect(state.status).toBe("failed");
  });

  it("marque un échec IPC et remet le suivi à zéro", () => {
    let state = runningState();
    state = teamRunReducer(state, { type: "fail" });
    expect(state.status).toBe("failed");
    state = teamRunReducer(state, { type: "reset" });
    expect(state).toBe(INITIAL_TEAM_RUN);
  });

  it("upsertTeamTask cumule le nombre de sessions", () => {
    const merged = upsertTeamTask([taskView({ sessions: 1 })], taskView({ sessions: 0 }));
    expect(merged).toHaveLength(1);
    expect(merged[0].sessions).toBe(1);
  });
});

// ------------------------------------------------------------------ diffs

describe("diffs d'une course d'équipe", () => {
  it("filtre sur l'origine exacte team:<run_id>", () => {
    const diffs = [
      diff("a", teamDiffOrigin("run-1")),
      diff("b", teamDiffOrigin("run-2")),
      diff("c", "agent:session-1"),
      diff("d", "team:"),
    ];
    expect(filterTeamPendingDiffs(diffs, "run-1").map((entry) => entry.id)).toEqual(["a"]);
    expect(filterTeamPendingDiffs(diffs, null)).toEqual([]);
    expect(teamDiffOrigin("run-1")).toBe(`${TEAM_DIFF_ORIGIN_PREFIX}run-1`);
  });
});
