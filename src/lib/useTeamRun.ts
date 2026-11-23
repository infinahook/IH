import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { api, type AppError, type TeamApprovalView, type TeamRunRequest } from "./api";
import { useTauriEvent } from "./events";
import { useAction } from "./hooks";
import {
  INITIAL_TEAM_RUN,
  readTeamEvent,
  teamRunReducer,
  type TeamRunState,
} from "./team";

export interface TeamRunController {
  /** Demande d'approbation affichée (modale). */
  approval: TeamApprovalView | null;
  state: TeamRunState;

  /** Un appel `team_request_approval` est en cours. */
  requesting: boolean;
  /** Une course est lancée (ou son lancement est en cours). */
  running: boolean;
  /** Un refus est en cours d'appel IPC. */
  refusing: boolean;

  launchError: AppError | null;
  runError: AppError | null;
  refuseError: AppError | null;
  liveError: AppError | null;
  dismissErrors: () => void;

  requestApproval: (request: TeamRunRequest) => Promise<void>;
  approve: () => Promise<void>;
  refuse: () => Promise<void>;
  closeApproval: () => void;
  reset: () => void;
}

/**
 * État complet du mode Équipe.
 *
 * `team_run` est une commande bloquante : elle ne rend son `run_id` qu'à la
 * fin, alors que les événements `ih://team-event` arrivent pendant la course.
 * Le réducteur adopte donc le `run_id` du premier événement, puis le résultat
 * final fusionne les tâches déjà vues sans doublon.
 */
export function useTeamRun(projectId: string | null): TeamRunController {
  const requestAction = useAction();
  const runAction = useAction();
  const refuseAction = useAction();

  const [approval, setApproval] = useState<TeamApprovalView | null>(null);
  const [state, dispatch] = useReducer(teamRunReducer, INITIAL_TEAM_RUN);

  /** Requête exacte couverte par l'approbation courante. */
  const pendingRequest = useRef<TeamRunRequest | null>(null);
  /** Vrai entre le démarrage d'une course et sa fin : ignore les événements tardifs. */
  const listening = useRef(false);
  const projectRef = useRef(projectId);
  projectRef.current = projectId;

  const reset = useCallback(() => {
    listening.current = false;
    pendingRequest.current = null;
    setApproval(null);
    dispatch({ type: "reset" });
    requestAction.reset();
    runAction.reset();
    refuseAction.reset();
  }, [requestAction, runAction, refuseAction]);

  const requestApproval = useCallback(
    async (request: TeamRunRequest) => {
      listening.current = false;
      pendingRequest.current = null;
      setApproval(null);
      dispatch({ type: "reset" });
      const scope = projectRef.current;
      const view = await requestAction.run(() => api.team.requestApproval(request));
      if (!view || projectRef.current !== scope) {
        return;
      }
      pendingRequest.current = request;
      setApproval(view);
    },
    [requestAction],
  );

  const approve = useCallback(async () => {
    const current = approval;
    const request = pendingRequest.current;
    if (!current || !request) {
      return;
    }
    const scope = projectRef.current;
    listening.current = true;
    setApproval(null);
    dispatch({ type: "start" });
    const result = await runAction.run(async () => {
      await api.approvalResolve(current.approval_id, "approve");
      return api.team.run(request, current.approval_id);
    });
    listening.current = false;
    if (!result) {
      dispatch({ type: "fail" });
      return;
    }
    if (projectRef.current !== scope) {
      dispatch({ type: "reset" });
      return;
    }
    pendingRequest.current = null;
    dispatch({ type: "result", result });
  }, [approval, runAction]);

  const refuse = useCallback(async () => {
    const current = approval;
    if (!current) {
      return;
    }
    const done = await refuseAction.run(async () => {
      await api.approvalResolve(current.approval_id, "deny");
      return true;
    });
    if (done) {
      pendingRequest.current = null;
      setApproval(null);
      dispatch({ type: "reset" });
    }
  }, [approval, refuseAction]);

  const closeApproval = useCallback(() => {
    // Fermer la modale ne consomme pas l'approbation : aucune course ne démarre.
    pendingRequest.current = null;
    setApproval(null);
  }, []);

  const liveError = useTauriEvent<unknown>("ih://team-event", (payload) => {
    if (!listening.current) {
      return;
    }
    const event = readTeamEvent(payload);
    if (event) {
      dispatch({ type: "event", event });
    }
  });

  const dismissErrors = useCallback(() => {
    requestAction.setError(null);
    runAction.setError(null);
    refuseAction.setError(null);
  }, [requestAction, runAction, refuseAction]);

  // Changement de projet : aucune course de l'ancien projet ne doit survivre.
  useEffect(() => {
    reset();
    // `reset` est stable ; seule la portée projet compte.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  return {
    approval,
    state,
    requesting: requestAction.busy,
    running: state.status === "running" || runAction.busy,
    refusing: refuseAction.busy,
    launchError: requestAction.error,
    runError: runAction.error,
    refuseError: refuseAction.error,
    liveError,
    dismissErrors,
    requestApproval,
    approve,
    refuse,
    closeApproval,
    reset,
  };
}
