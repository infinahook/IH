import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import {
  api,
  toAppError,
  type AgentAdapterReport,
  type AgentApprovalView,
  type AgentSessionDetail,
  type AppError,
} from "./api";
import {
  INITIAL_AGENT_EVENTS,
  INITIAL_AGENT_LAUNCH,
  MAX_PERSISTED_AGENT_EVENTS,
  agentEventReducer,
  agentLaunchBlockedReason,
  agentLaunchReducer,
  isActiveAgentPhase,
  resolveAdapterSelection,
  resolveAgentPhase,
  type AgentEvent,
  type AgentLaunchBlocked,
  type AgentLiveEvent,
  type AgentPhase,
} from "./agent";
import { useTauriEvent } from "./events";
import { useAction, useAsync } from "./hooks";
import { useApp } from "./store";

export interface AgentNotice {
  text: string;
  tone: "info" | "success" | "warning" | "error";
}

export interface AgentSessionController {
  // Adaptateurs réellement détectés par `agent_adapters`.
  adapters: AgentAdapterReport[];
  adaptersLoading: boolean;
  adaptersError: AppError | null;
  reloadAdapters: () => void;
  selectedAgentId: string;
  selectedAdapter: AgentAdapterReport | null;
  selectAdapter: (id: string) => void;

  // Tâche et lancement.
  task: string;
  setTask: (value: string) => void;
  launchBlocked: AgentLaunchBlocked;
  canLaunch: boolean;
  /** Confirmation explicite exigée pour exécuter un agent sans sandbox. */
  unsandboxedAck: boolean;
  setUnsandboxedAck: (value: boolean) => void;
  approval: AgentApprovalView | null;
  phase: AgentPhase;
  active: boolean;
  sessionId: string | null;
  /** Une demande d'approbation ou un démarrage est en cours d'appel IPC. */
  launching: boolean;
  /** Une annulation de session est en cours d'appel IPC. */
  cancelling: boolean;
  /** Un refus d'approbation est en cours d'appel IPC. */
  refusing: boolean;

  // Flux d'événements de la session suivie.
  events: AgentEvent[];
  steps: number;
  outputBytes: number;
  bounded: boolean;
  summary: string | null;
  files: string[];
  changesPrepared: number | null;
  sessionError: string | null;

  // Détail persisté (après redémarrage notamment).
  detail: AgentSessionDetail | null;
  detailLoading: boolean;
  detailError: AppError | null;

  // Sessions récentes du projet.
  sessions: AgentSessionDetail[];
  sessionsLoading: boolean;
  sessionsError: AppError | null;
  reloadSessions: () => void;

  // Erreurs IPC et messages, jamais avalés.
  launchError: AppError | null;
  refuseError: AppError | null;
  cancelError: AppError | null;
  dismissErrors: () => void;
  liveError: AppError | null;
  notice: AgentNotice | null;
  dismissNotice: () => void;

  // Actions réelles.
  requestApproval: () => Promise<void>;
  approve: () => Promise<void>;
  refuse: () => Promise<void>;
  closeApproval: () => void;
  cancelSession: () => Promise<boolean>;
  relaunch: () => Promise<void>;
  loadSession: (id: string) => Promise<void>;
  clearSession: () => void;
}

const TERMINAL_KINDS: ReadonlySet<string> = new Set(["completed", "cancelled", "error"]);

/**
 * État complet du mode Agent : adaptateurs, sessions persistées et flux temps
 * réel. Toutes les commandes sont réelles ; les redéductions pures vivent dans
 * `agent.ts` et restent testables sans WebView.
 */
export function useAgentSession(projectId: string | null): AgentSessionController {
  const [unsandboxedAck, setUnsandboxedAckState] = useState(false);
  const unsandboxedAckRef = useRef(false);
  const setUnsandboxedAck = useCallback((value: boolean) => {
    unsandboxedAckRef.current = value;
    setUnsandboxedAckState(value);
  }, []);
  const { t } = useApp();
  const adapters = useAsync<AgentAdapterReport[]>(() => api.agent.adapters(), []);
  const sessions = useAsync<AgentSessionDetail[]>(
    async () => (projectId ? api.agent.sessions(projectId) : []),
    [projectId],
  );
  const launchAction = useAction();
  const refuseAction = useAction();
  const cancelAction = useAction();

  const [selectedAgentId, setSelectedAgentId] = useState("");
  const [task, setTask] = useState("");
  const [launch, dispatchLaunch] = useReducer(agentLaunchReducer, INITIAL_AGENT_LAUNCH);
  const [stream, dispatchStream] = useReducer(agentEventReducer, INITIAL_AGENT_EVENTS);
  const [detail, setDetail] = useState<AgentSessionDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<AgentNotice | null>(null);

  const projectRef = useRef(projectId);
  projectRef.current = projectId;
  /** Session sélectionnée, mise à jour de façon synchrone (course événements). */
  const sessionRef = useRef<string | null>(null);
  const detailToken = useRef(0);

  const selectedAdapter = useMemo(
    () => resolveAdapterSelection(adapters.data ?? [], selectedAgentId),
    [adapters.data, selectedAgentId],
  );

  const selectAdapter = useCallback(
    (id: string) => {
      const target = (adapters.data ?? []).find((entry) => entry.id === id);
      if (!target || !target.detected) {
        // Un adaptateur introuvable n'est jamais sélectionnable.
        return;
      }
      setSelectedAgentId(id);
    },
    [adapters.data],
  );

  /**
   * Relit une session persistée : détail + événements. Le flux temps réel déjà
   * reçu est conservé et fusionné par `agentEventReducer` (clé `seq`).
   */
  const hydrate = useCallback(async (sessionId: string, syncFields: boolean) => {
    const token = detailToken.current + 1;
    detailToken.current = token;
    setDetailLoading(true);
    setDetailError(null);
    try {
      const [loaded, events] = await Promise.all([
        api.agent.detail(sessionId),
        api.agent.events(sessionId, MAX_PERSISTED_AGENT_EVENTS),
      ]);
      if (detailToken.current !== token || sessionRef.current !== sessionId) {
        // La sélection a changé pendant la lecture : ne pas écraser le panneau.
        return;
      }
      setDetail(loaded);
      dispatchStream({ type: "history", sessionId, events });
      if (syncFields) {
        setTask(loaded.task ?? "");
        if (loaded.adapter) {
          setSelectedAgentId(loaded.adapter);
        }
      }
    } catch (reason) {
      if (detailToken.current === token) {
        setDetailError(toAppError(reason));
      }
    } finally {
      if (detailToken.current === token) {
        setDetailLoading(false);
      }
    }
  }, []);

  const requestApprovalFor = useCallback(
    async (agentId: string, text: string) => {
      const scope = projectRef.current;
      const trimmed = text.trim();
      if (!scope || !agentId || trimmed.length === 0) {
        return;
      }
      setNotice(null);
      setDetail(null);
      setDetailError(null);
      setDetailLoading(false);
      detailToken.current += 1;
      sessionRef.current = null;
      dispatchStream({ type: "select", sessionId: null });
      dispatchLaunch({ type: "reset" });
      const approval = await launchAction.run(() =>
        api.agent.requestApproval(scope, agentId, trimmed, unsandboxedAckRef.current),
      );
      if (!approval || projectRef.current !== scope) {
        return;
      }
      dispatchLaunch({ type: "request", approval, agentId, task: trimmed });
    },
    [launchAction],
  );

  const requestApproval = useCallback(async () => {
    if (!selectedAdapter) {
      return;
    }
    await requestApprovalFor(selectedAdapter.id, task);
  }, [selectedAdapter, task, requestApprovalFor]);

  const approve = useCallback(async () => {
    const scope = projectRef.current;
    const approval = launch.approval;
    if (!scope || !approval) {
      return;
    }
    dispatchLaunch({ type: "approve" });
    const started = await launchAction.run(async () => {
      await api.approvalResolve(approval.approval_id, "approve");
      return api.agent.start(scope, null, approval.agent_id, launch.task, approval.approval_id);
    });
    if (!started) {
      // L'approbation peut avoir été consommée : on repart d'une demande neuve.
      dispatchLaunch({ type: "reset" });
      sessionRef.current = null;
      dispatchStream({ type: "select", sessionId: null });
      return;
    }
    if (projectRef.current !== scope) {
      void api.agent.cancel(started.session_id).catch(() => undefined);
      return;
    }
    dispatchLaunch({ type: "started", session: started });
    sessionRef.current = started.session_id;
    dispatchStream({ type: "select", sessionId: started.session_id });
    setDetail(null);
    setNotice({
      text: t("agent_mode.session_started", { id: started.session_id }),
      tone: "success",
    });
    sessions.reload();
    void hydrate(started.session_id, false);
  }, [launch.approval, launch.task, launchAction, hydrate, sessions, t]);

  const refuse = useCallback(async () => {
    const approval = launch.approval;
    if (!approval) {
      return;
    }
    const refused = await refuseAction.run(async () => {
      await api.agent.refuse(approval.approval_id);
      return true;
    });
    if (refused) {
      dispatchLaunch({ type: "refuse" });
      setNotice({ text: t("agent_mode.approval_refused"), tone: "info" });
    }
  }, [launch.approval, refuseAction, t]);

  const closeApproval = useCallback(() => {
    // Annuler ferme la demande sans rien lancer et sans appel IPC.
    dispatchLaunch({ type: "dismiss" });
  }, []);

  const cancelSession = useCallback(async () => {
    const target =
      sessionRef.current ??
      stream.sessionId ??
      launch.session?.session_id ??
      detail?.session.id ??
      null;
    if (!target) {
      return false;
    }
    const done = await cancelAction.run(async () => {
      await api.agent.cancel(target);
      return true;
    });
    if (done !== true) {
      return false;
    }
    setNotice({ text: t("agent_mode.cancel_requested"), tone: "info" });
    sessions.reload();
    return true;
  }, [stream.sessionId, launch.session, detail, cancelAction, sessions, t]);

  const relaunch = useCallback(async () => {
    const agentId = detail?.adapter ?? launch.agentId ?? selectedAdapter?.id ?? "";
    const text = task.trim().length > 0 ? task : detail?.task ?? "";
    if (!agentId || text.trim().length === 0) {
      return;
    }
    await requestApprovalFor(agentId, text);
  }, [detail, launch.agentId, selectedAdapter, task, requestApprovalFor]);

  const loadSession = useCallback(
    async (id: string) => {
      setNotice(null);
      dispatchLaunch({ type: "reset" });
      sessionRef.current = id;
      dispatchStream({ type: "select", sessionId: id });
      setDetail(null);
      setDetailError(null);
      await hydrate(id, true);
    },
    [hydrate],
  );

  const clearSession = useCallback(() => {
    detailToken.current += 1;
    sessionRef.current = null;
    dispatchLaunch({ type: "reset" });
    dispatchStream({ type: "reset" });
    setDetail(null);
    setDetailError(null);
    setDetailLoading(false);
    setNotice(null);
  }, []);

  const liveError = useTauriEvent<AgentLiveEvent>("ih://agent-event", (payload) => {
    dispatchStream({ type: "live", event: payload });
    if (TERMINAL_KINDS.has(payload.kind) && sessionRef.current === payload.session_id) {
      // Le noyau a persisté résumé et fichiers juste avant l'émission.
      void hydrate(payload.session_id, false);
      sessions.reload();
    }
  });

  // Changement de projet : aucune session de l'ancien projet ne doit survivre.
  useEffect(() => {
    detailToken.current += 1;
    sessionRef.current = null;
    setDetail(null);
    setDetailError(null);
    setDetailLoading(false);
    setTask("");
    setSelectedAgentId("");
    setNotice(null);
    dispatchLaunch({ type: "reset" });
    dispatchStream({ type: "reset" });
    launchAction.reset();
    refuseAction.reset();
    cancelAction.reset();
    // `reset` et les actions sont stables ; seule la portée projet compte.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  const phase = resolveAgentPhase(
    stream.phase,
    detail?.session.state ?? null,
    launch.phase,
  );
  const active = isActiveAgentPhase(phase);
  const launchBlocked = agentLaunchBlockedReason({
    adapter: selectedAdapter,
    task,
    active,
    busy: launchAction.busy,
  });

  const dismissErrors = useCallback(() => {
    launchAction.setError(null);
    refuseAction.setError(null);
    cancelAction.setError(null);
  }, [launchAction, refuseAction, cancelAction]);

  const sessionId = stream.sessionId ?? launch.session?.session_id ?? detail?.session.id ?? null;

  return {
    adapters: adapters.data ?? [],
    adaptersLoading: adapters.loading,
    adaptersError: adapters.error,
    reloadAdapters: adapters.reload,
    selectedAgentId: selectedAdapter?.id ?? "",
    selectedAdapter,
    selectAdapter,
    task,
    setTask,
    launchBlocked,
    canLaunch: launchBlocked === null,
    unsandboxedAck,
    setUnsandboxedAck,
    approval: launch.approval,
    phase,
    active,
    sessionId,
    launching: launchAction.busy,
    cancelling: cancelAction.busy,
    refusing: refuseAction.busy,
    events: stream.events,
    steps: stream.events.length,
    outputBytes: stream.outputBytes,
    bounded: stream.bounded,
    summary: stream.summary ?? detail?.summary ?? null,
    files: stream.files.length > 0 ? stream.files : detail?.files ?? [],
    changesPrepared: stream.changesPrepared,
    sessionError: stream.error,
    detail,
    detailLoading,
    detailError,
    sessions: (sessions.data ?? []).filter(
      (entry) => projectId !== null && entry.session.project_id === projectId,
    ),
    sessionsLoading: sessions.loading,
    sessionsError: sessions.error,
    reloadSessions: sessions.reload,
    launchError: launchAction.error,
    refuseError: refuseAction.error,
    cancelError: cancelAction.error,
    dismissErrors,
    liveError,
    notice,
    dismissNotice: () => setNotice(null),
    requestApproval,
    approve,
    refuse,
    closeApproval,
    cancelSession,
    relaunch,
    loadSession,
    clearSession,
  };
}
