import { describe, expect, it } from "vitest";
import type { AgentAdapterReport, AgentApprovalView, AgentEventRow, PendingDiff } from "./api";
import {
  AGENT_DIFF_ORIGIN_PREFIX,
  INITIAL_AGENT_EVENTS,
  INITIAL_AGENT_LAUNCH,
  MAX_AGENT_EVENTS,
  MAX_AGENT_OUTPUT_BYTES,
  agentDiffOrigin,
  agentEventLabelKey,
  agentEventReducer,
  agentLaunchBlockedReason,
  agentLaunchReducer,
  agentOutputEventCount,
  agentPhaseLabelKey,
  canLaunchAgent,
  filterAgentPendingDiffs,
  isActiveAgentPhase,
  isAdapterSelectable,
  networkLabelKey,
  requiresUnsandboxedAck,
  sandboxLabelKey,
  securityLabelKey,
  isTerminalAgentPhase,
  mapAgentState,
  pickDefaultAdapter,
  resolveAdapterSelection,
  resolveAgentPhase,
  selectableAdapters,
  type AgentLaunchState,
  type AgentEventStream,
  type AgentLiveEvent,
} from "./agent";

// ---------------------------------------------------------------- fabriques

function row(
  seq: number,
  kind: string,
  payload: unknown = {},
  sessionId = "session-1",
): AgentEventRow {
  return {
    id: seq + 1,
    session_id: sessionId,
    seq,
    kind,
    payload,
    created_at: "2026-01-01T00:00:00Z",
  };
}

function live(
  seq: number,
  kind: string,
  payload: unknown = {},
  sessionId = "session-1",
): AgentLiveEvent {
  return { session_id: sessionId, seq, kind, payload };
}

function output(seq: number, data: string, stream: "stdout" | "stderr" = "stdout"): AgentLiveEvent {
  return live(seq, "output", { stream, data });
}

function selected(sessionId = "session-1"): AgentEventStream {
  return agentEventReducer(INITIAL_AGENT_EVENTS, { type: "select", sessionId });
}

function liveInto(state: AgentEventStream, event: AgentLiveEvent): AgentEventStream {
  return agentEventReducer(state, { type: "live", event });
}

function adapter(id: string, detected: boolean): AgentAdapterReport {
  return {
    id,
    label: id,
    kind: "claude_code",
    program: id,
    detected,
    path: detected ? `/usr/bin/${id}` : null,
    version: detected ? "1.0.0" : null,
    functional: detected,
    security: "isolated_network",
    sandbox: {
      level: "isolated_network",
      network: true,
      sandboxed: true,
      engine: "bubblewrap",
      engine_ready: true,
      error: null,
      warnings: [],
    },
    capabilities: [],
    diagnostic: detected ? "exécutable trouvé" : "introuvable dans le PATH",
  };
}

function approval(id = "approval-1", agentId = "agent-ok"): AgentApprovalView {
  return {
    approval_id: id,
    agent_id: agentId,
    label: "Claude Code",
    command: "claude -p 'corrige les tests'",
    workdir: "/home/infina/projects/demo",
    risk: "high",
    fingerprint: "abc123",
    detail: "Commande : claude -p …",
  };
}

function pendingDiff(id: string, origin: string, deletions = false): PendingDiff {
  return {
    id,
    project_id: "project-1",
    path: `src/${id}.ts`,
    origin,
    delete: deletions,
    created_at: "2026-01-01T00:00:00Z",
  };
}

// ------------------------------------------------------------------- tri

describe("réduction des événements d'agent", () => {
  it("ordonne les événements par seq, quel que soit l'ordre d'arrivée", () => {
    let state = selected();
    state = liveInto(state, output(2, "c"));
    state = liveInto(state, output(0, "a"));
    state = liveInto(state, output(1, "b"));
    expect(state.events.map((event) => event.seq)).toEqual([0, 1, 2]);
  });

  it("ignore un doublon déjà vu et renvoie l'état inchangé", () => {
    let state = selected();
    state = liveInto(state, output(7, "une seule fois"));
    const after = liveInto(state, output(7, "une seule fois"));
    expect(after).toBe(state);
    expect(after.events).toHaveLength(1);
  });

  it("conserve l'ordre d'arrivée quand deux événements partagent seq = -1", () => {
    let state = selected();
    state = liveInto(state, live(-1, "started", { agent_id: "agent-ok" }));
    state = liveInto(
      state,
      live(-1, "completed", { summary: "fini", files: [], changes_prepared: 0, error: null }),
    );
    const final = liveInto(
      state,
      live(-1, "completed", { summary: "fini", files: [], changes_prepared: 0, error: null }),
    );
    expect(final).toBe(state);
    expect(state.events.map((event) => event.kind)).toEqual(["started", "completed"]);
    expect(state.phase).toBe("completed");
  });

  it("ignore un événement temps réel d'une autre session", () => {
    let state = selected("session-1");
    state = liveInto(state, output(0, "à moi"));
    state = liveInto(state, output(1, "voisine", "stdout"));
    const ignored = liveInto(state, {
      session_id: "session-2",
      seq: 0,
      kind: "output",
      payload: { stream: "stdout", data: "intruse" },
    });
    expect(ignored).toBe(state);
    expect(state.sessionId).toBe("session-1");
    expect(state.events).toHaveLength(2);
  });

  it("ignore un événement temps réel quand aucune session n'est suivie", () => {
    const state = liveInto(INITIAL_AGENT_EVENTS, output(0, "orphelin"));
    expect(state).toBe(INITIAL_AGENT_EVENTS);
    expect(state.events).toHaveLength(0);
  });

  it("ignore les lignes d'historique d'une autre session", () => {
    let state = selected("session-1");
    state = agentEventReducer(state, {
      type: "history",
      sessionId: "session-1",
      events: [row(0, "output", { stream: "stdout", data: "ok" }), row(0, "output", {}, "session-2")],
    });
    expect(state.events).toHaveLength(1);
    expect(state.events[0].payload).toEqual({ stream: "stdout", data: "ok" });
  });
});

// ------------------------------------------------------------------ bornes

describe("bornes mémoire du flux d'agent", () => {
  it("conserve au plus 1500 événements, les plus récents", () => {
    let state = selected();
    for (let seq = 0; seq < MAX_AGENT_EVENTS + 100; seq += 1) {
      state = liveInto(state, output(seq, `ligne ${seq}\n`));
    }
    expect(state.events).toHaveLength(MAX_AGENT_EVENTS);
    expect(state.events[0].seq).toBe(100);
    expect(state.events[state.events.length - 1].seq).toBe(MAX_AGENT_EVENTS + 99);
    expect(state.bounded).toBe(true);
  });

  it("borne le texte cumulé à 256 Kio en gardant les plus récents", () => {
    const chunk = "x".repeat(1024);
    let state = selected();
    for (let seq = 0; seq < 300; seq += 1) {
      state = liveInto(state, output(seq, chunk));
    }
    expect(state.outputBytes).toBeLessThanOrEqual(MAX_AGENT_OUTPUT_BYTES);
    expect(state.outputBytes).toBe(MAX_AGENT_OUTPUT_BYTES);
    expect(state.events).toHaveLength(256);
    expect(state.events[0].seq).toBe(44);
    expect(state.bounded).toBe(true);
  });

  it("conserve la phase dérivée même après troncature", () => {
    let state = selected();
    state = liveInto(state, live(0, "state", { state: "running" }));
    for (let seq = 1; seq <= MAX_AGENT_EVENTS + 20; seq += 1) {
      state = liveInto(state, output(seq, "sortie\n"));
    }
    expect(state.phase).toBe("running");
    expect(state.events.some((event) => event.kind === "state")).toBe(false);
  });

  it("conserve résumé, fichiers et phase terminale même si l'événement est retiré", () => {
    let state = liveInto(
      selected(),
      live(-1, "completed", {
        summary: "1 fichier modifié",
        files: ["src/a.ts"],
        changes_prepared: 1,
        error: null,
      }),
    );
    for (let seq = 0; seq < MAX_AGENT_EVENTS + 10; seq += 1) {
      state = liveInto(state, output(seq, "bruit\n"));
    }
    expect(state.events.some((event) => event.kind === "completed")).toBe(false);
    expect(state.summary).toBe("1 fichier modifié");
    expect(state.files).toEqual(["src/a.ts"]);
    expect(state.changesPrepared).toBe(1);
    expect(state.phase).toBe("completed");
  });

  it("ne marque pas comme borné un flux resté sous les limites", () => {
    let state = selected();
    state = liveInto(state, output(0, "court"));
    expect(state.bounded).toBe(false);
  });
});

// ------------------------------------------------------ fusion historique

describe("fusion historique et temps réel", () => {
  it("fusionne agent_session_events et ih://agent-event sans doublon", () => {
    let state = selected();
    state = liveInto(state, output(1, "temps réel 1"));
    state = liveInto(state, output(2, "temps réel 2"));
    state = agentEventReducer(state, {
      type: "history",
      sessionId: "session-1",
      events: [
        row(0, "state", { state: "running" }),
        row(1, "output", { stream: "stdout", data: "temps réel 1" }),
        row(2, "output", { stream: "stdout", data: "temps réel 2" }),
      ],
    });
    expect(state.events.map((event) => event.seq)).toEqual([0, 1, 2]);
    expect(state.events).toHaveLength(3);
  });

  it("accepte un historique rejoué sans dupliquer les événements déjà affichés", () => {
    let state = selected();
    const history = [row(0, "output", { stream: "stdout", data: "a" }), row(1, "output", { stream: "stdout", data: "b" })];
    state = agentEventReducer(state, { type: "history", sessionId: "session-1", events: history });
    state = agentEventReducer(state, { type: "history", sessionId: "session-1", events: history });
    expect(state.events).toHaveLength(2);
  });

  it("ajoute un événement temps réel arrivé pendant le chargement de l'historique", () => {
    let state = selected();
    state = agentEventReducer(state, {
      type: "history",
      sessionId: "session-1",
      events: [row(0, "output", { stream: "stdout", data: "avant" })],
    });
    state = liveInto(state, output(1, "pendant"));
    expect(state.events.map((event) => event.seq)).toEqual([0, 1]);
  });

  it("réinitialise le flux quand on sélectionne une autre session", () => {
    let state = selected("session-1");
    state = liveInto(state, output(0, "ancienne"));
    state = selected("session-2");
    expect(state.sessionId).toBe("session-2");
    expect(state.events).toHaveLength(0);
    expect(state.seen.size).toBe(0);
  });
});

// ------------------------------------------------------------------ états

describe("états dérivés des événements", () => {
  it("passe en running dès l'événement started", () => {
    let state = selected();
    state = liveInto(state, live(-1, "started", { agent_id: "agent-ok", workdir: "/tmp/wt" }));
    expect(state.phase).toBe("running");
    expect(isActiveAgentPhase(state.phase ?? "idle")).toBe(true);
  });

  it("suit les états du noyau", () => {
    let state = selected();
    state = liveInto(state, live(0, "state", { state: "starting" }));
    expect(state.phase).toBe("starting");
    state = liveInto(state, live(1, "state", { state: "waiting_approval" }));
    expect(state.phase).toBe("waiting_approval");
    state = liveInto(state, live(2, "state", { state: "running" }));
    expect(state.phase).toBe("running");
  });

  it("calcule l'état terminal à partir des kinds completed, cancelled et error", () => {
    const completed = liveInto(
      selected(),
      live(-1, "completed", {
        summary: "2 fichier(s) modifié(s)",
        files: ["src/a.ts", "src/b.ts"],
        changes_prepared: 2,
        error: null,
      }),
    );
    expect(completed.phase).toBe("completed");
    expect(completed.summary).toBe("2 fichier(s) modifié(s)");
    expect(completed.files).toEqual(["src/a.ts", "src/b.ts"]);
    expect(completed.changesPrepared).toBe(2);
    expect(completed.error).toBeNull();
    expect(isTerminalAgentPhase(completed.phase ?? "idle")).toBe(true);

    const cancelled = liveInto(selected(), live(-1, "cancelled", { summary: "session annulée" }));
    expect(cancelled.phase).toBe("cancelled");

    const failed = liveInto(
      selected(),
      live(-1, "error", { summary: "échec", files: [], changes_prepared: 0, error: "code 1" }),
    );
    expect(failed.phase).toBe("failed");
    expect(failed.error).toBe("code 1");
  });

  it("ne rouvre jamais une session terminée sur un état tardif", () => {
    let state = liveInto(selected(), live(-1, "completed", { summary: "fini" }));
    state = liveInto(state, live(5, "state", { state: "running" }));
    expect(state.phase).toBe("completed");
  });

  it("ignore une charge payload malformée", () => {
    let state = selected();
    state = liveInto(state, live(0, "output", "pas un objet"));
    state = liveInto(state, live(1, "state", { state: 42 }));
    expect(state.events).toHaveLength(2);
    expect(state.phase).toBeNull();
    expect(state.outputBytes).toBe(0);
  });

  it("résout la phase affichée à partir de la session persistée et du lancement", () => {
    expect(resolveAgentPhase(null, "running", "idle")).toBe("running");
    expect(resolveAgentPhase(null, "completed", "running")).toBe("completed");
    expect(resolveAgentPhase("cancelled", "running", "running")).toBe("cancelled");
    expect(resolveAgentPhase(null, null, "approving")).toBe("approving");
    expect(resolveAgentPhase(null, null, "idle")).toBe("idle");
    expect(mapAgentState("ready")).toBe("starting");
    expect(mapAgentState("inconnu")).toBeNull();
    expect(agentPhaseLabelKey("waiting_approval")).toBe("agent_mode.phase_waiting_approval");
  });

  it("compte les événements de sortie et nomme les kinds connus", () => {
    let state = selected();
    state = liveInto(state, output(0, "a"));
    state = liveInto(state, output(1, "b", "stderr"));
    state = liveInto(state, live(2, "state", { state: "running" }));
    expect(agentOutputEventCount(state.events)).toBe(2);
    expect(agentEventLabelKey("output")).toBe("agent_mode.kind_output");
    expect(agentEventLabelKey("kind-inconnu")).toBeNull();
  });
});

// ------------------------------------------------------- adaptateurs

describe("sélection des adaptateurs", () => {
  const missing = adapter("agent-missing", false);
  const detected = adapter("agent-ok", true);

  it("ne rend sélectionnable qu'un adaptateur réellement détecté", () => {
    expect(isAdapterSelectable(detected)).toBe(true);
    expect(isAdapterSelectable(missing)).toBe(false);
    expect(selectableAdapters([missing, detected])).toEqual([detected]);
    expect(pickDefaultAdapter([missing, detected])).toEqual(detected);
    expect(pickDefaultAdapter([missing])).toBeNull();
  });

  it("retombe sur le premier adaptateur détecté si la sélection est introuvable", () => {
    expect(resolveAdapterSelection([missing, detected], "agent-missing")).toEqual(detected);
    expect(resolveAdapterSelection([missing, detected], detected.id)).toEqual(detected);
    expect(resolveAdapterSelection([missing], "agent-missing")).toBeNull();
  });

  it("bloque le lancement sans adaptateur détecté, sans tâche ou pendant une session", () => {
    expect(agentLaunchBlockedReason({ adapter: null, task: "faire", active: false, busy: false })).toBe(
      "adapter",
    );
    expect(
      agentLaunchBlockedReason({ adapter: missing, task: "faire", active: false, busy: false }),
    ).toBe("adapter");
    expect(
      agentLaunchBlockedReason({ adapter: detected, task: "   ", active: false, busy: false }),
    ).toBe("task");
    expect(
      agentLaunchBlockedReason({ adapter: detected, task: "faire", active: true, busy: false }),
    ).toBe("active");
    expect(
      agentLaunchBlockedReason({ adapter: detected, task: "faire", active: false, busy: true }),
    ).toBe("busy");
    expect(
      canLaunchAgent({ adapter: detected, task: "faire", active: false, busy: false }),
    ).toBe(true);
    expect(canLaunchAgent({ adapter: missing, task: "faire", active: false, busy: false })).toBe(
      false,
    );
  });
});

// --------------------------------------------------- flux d'approbation

describe("flux d'approbation", () => {
  it("enchaîne lancer → autoriser → running", () => {
    let state: AgentLaunchState = INITIAL_AGENT_LAUNCH;
    state = agentLaunchReducer(state, {
      type: "request",
      approval: approval(),
      agentId: "agent-ok",
      task: "corrige les tests",
    });
    expect(state.phase).toBe("approving");
    expect(state.approval?.approval_id).toBe("approval-1");
    expect(state.task).toBe("corrige les tests");

    state = agentLaunchReducer(state, { type: "approve" });
    expect(state.phase).toBe("starting");
    expect(state.approval?.approval_id).toBe("approval-1");

    state = agentLaunchReducer(state, {
      type: "started",
      session: { session_id: "session-1", workdir: "/tmp/wt", agent_id: "agent-ok" },
    });
    expect(state.phase).toBe("running");
    expect(state.session?.session_id).toBe("session-1");
    expect(state.approval).toBeNull();
    expect(isActiveAgentPhase(state.phase)).toBe(true);
  });

  it("refuser ne lance rien et revient à l'état initial", () => {
    let state = agentLaunchReducer(INITIAL_AGENT_LAUNCH, {
      type: "request",
      approval: approval(),
      agentId: "agent-ok",
      task: "tâche",
    });
    state = agentLaunchReducer(state, { type: "refuse" });
    expect(state.phase).toBe("idle");
    expect(state.refused).toBe(true);
    expect(state.approval).toBeNull();
    expect(state.session).toBeNull();
    expect(isActiveAgentPhase(state.phase)).toBe(false);
  });

  it("annuler ferme la demande sans lancer de session", () => {
    let state = agentLaunchReducer(INITIAL_AGENT_LAUNCH, {
      type: "request",
      approval: approval(),
      agentId: "agent-ok",
      task: "tâche",
    });
    state = agentLaunchReducer(state, { type: "dismiss" });
    expect(state.phase).toBe("idle");
    expect(state.approval).toBeNull();
    expect(state.session).toBeNull();
    expect(state.refused).toBe(false);
  });

  it("refuse les transitions hors séquence", () => {
    expect(agentLaunchReducer(INITIAL_AGENT_LAUNCH, { type: "approve" }).phase).toBe("idle");
    expect(
      agentLaunchReducer(INITIAL_AGENT_LAUNCH, {
        type: "started",
        session: { session_id: "session-1", workdir: "/tmp", agent_id: "agent-ok" },
      }).phase,
    ).toBe("idle");
    const approving = agentLaunchReducer(INITIAL_AGENT_LAUNCH, {
      type: "request",
      approval: approval(),
      agentId: "agent-ok",
      task: "tâche",
    });
    const again = agentLaunchReducer(approving, {
      type: "request",
      approval: approval("approval-2"),
      agentId: "agent-ok",
      task: "autre",
    });
    expect(again).toBe(approving);
  });
});

// ------------------------------------------------------------ diffs ag

describe("revue des modifications d'une session", () => {
  it("filtre les diffs en attente sur l'origine exacte de la session", () => {
    const sessionId = "session-1";
    const diffs = [
      pendingDiff("a", agentDiffOrigin(sessionId)),
      pendingDiff("b", `${AGENT_DIFF_ORIGIN_PREFIX}session-2`),
      pendingDiff("c", "editor"),
    ];
    const filtered = filterAgentPendingDiffs(diffs, sessionId);
    expect(filtered.map((entry) => entry.id)).toEqual(["a"]);
    expect(agentDiffOrigin("session-1")).toBe("agent:session-1");
    expect(filterAgentPendingDiffs(diffs, null)).toEqual([]);
  });

describe("présentation des niveaux de sécurité", () => {
  it("mappe chaque niveau sur son badge i18n", () => {
    expect(securityLabelKey("isolated")).toBe("agent_security.level_isolated");
    expect(securityLabelKey("isolated_network")).toBe(
      "agent_security.level_isolated_network",
    );
    expect(securityLabelKey("none")).toBe("agent_security.level_none");
    // Niveau absent : défaut documenté, jamais un niveau plus permissif.
    expect(securityLabelKey(undefined)).toBe("agent_security.level_isolated_network");
  });

  it("déduit l'état réseau du niveau réel", () => {
    expect(networkLabelKey("isolated", true)).toBe("agent_security.network_off");
    expect(networkLabelKey("isolated_network", true)).toBe("agent_security.network_on");
    expect(networkLabelKey("isolated_network", false)).toBe("agent_security.network_off");
  });

  it("signale une sandbox indisponible et n'en affiche pas pour un niveau non isolé", () => {
    expect(sandboxLabelKey({ level: "isolated_network", engine_ready: true })).toBe(
      "agent_security.sandbox_ready",
    );
    expect(sandboxLabelKey({ level: "isolated", engine_ready: false })).toBe(
      "agent_security.sandbox_unavailable",
    );
    expect(sandboxLabelKey({ level: "none", engine_ready: false })).toBe("");
    expect(sandboxLabelKey(undefined)).toBe("");
  });

  it("n'exige la confirmation hors sandbox que pour le niveau non isolé", () => {
    expect(requiresUnsandboxedAck("none")).toBe(true);
    expect(requiresUnsandboxedAck("isolated")).toBe(false);
    expect(requiresUnsandboxedAck("isolated_network")).toBe(false);
    expect(requiresUnsandboxedAck(undefined)).toBe(false);
  });
});
});
