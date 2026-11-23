import { useEffect, useMemo, useState } from "react";
import { Check, FileCode2, FileDiff, Play, RefreshCw, ShieldAlert, Users, X } from "lucide-react";
import {
  api,
  toAppError,
  type AgentAdapterReport,
  type AppError,
  type DiffPreview,
} from "../lib/api";
import { selectableAdapters } from "../lib/agent";
import { shortHash } from "../lib/format";
import {
  buildTeamRequest,
  filterTeamPendingDiffs,
  teamFieldErrorKey,
  teamRoleLabelKey,
  teamStatusLabelKey,
  teamTaskStateLabelKey,
  type TeamForm,
  type TeamRole,
  type TeamRunStatus,
} from "../lib/team";
import { useApp } from "../lib/store";
import type { TeamRunController } from "../lib/useTeamRun";
import type { WorkspaceController } from "../lib/useWorkspace";
import { Badge, taskStateTone, type BadgeTone } from "./Badge";
import { DiffView } from "./DiffView";
import { EmptyState, Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";
import { Field } from "./Field";
import { Modal } from "./Modal";

function runTone(status: TeamRunStatus): BadgeTone {
  switch (status) {
    case "running":
      return "accent";
    case "completed":
      return "success";
    case "failed":
      return "danger";
    default:
      return "neutral";
  }
}

function roleTone(role: TeamRole | null): BadgeTone {
  switch (role) {
    case "lead":
      return "accent";
    case "worker":
      return "info";
    case "reviewer":
      return "warning";
    default:
      return "neutral";
  }
}

/**
 * Sous-mode Équipe du panneau Agent : composition de l'équipe, approbation
 * unique, suivi temps réel des tâches et revue des diffs `team:<run_id>`.
 * Aucun appel IPC n'est simulé ; les erreurs gardent leur code noyau.
 */
export function TeamPanel({
  projectId,
  adapters,
  adaptersLoading,
  adaptersError,
  onReloadAdapters,
  controller,
  workspace,
}: {
  projectId: string;
  adapters: AgentAdapterReport[];
  adaptersLoading: boolean;
  adaptersError: AppError | null;
  onReloadAdapters: () => void;
  controller: TeamRunController;
  workspace: WorkspaceController;
}) {
  const { t } = useApp();
  const [goal, setGoal] = useState("");
  const [lead, setLead] = useState("");
  const [workers, setWorkers] = useState<string[]>([]);
  const [reviewer, setReviewer] = useState("");
  const [workItems, setWorkItems] = useState(2);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [diffPreview, setDiffPreview] = useState<DiffPreview | null>(null);
  const [diffPreviewError, setDiffPreviewError] = useState<AppError | null>(null);
  const [diffPreviewBusy, setDiffPreviewBusy] = useState(false);

  const detected = useMemo(() => selectableAdapters(adapters), [adapters]);

  // Sélection par défaut : premier adaptateur détecté, sans jamais garder un
  // identifiant qui aurait disparu après une nouvelle détection.
  useEffect(() => {
    if (detected.length === 0) {
      return;
    }
    setLead((current) =>
      detected.some((adapter) => adapter.id === current) ? current : detected[0].id,
    );
    setWorkers((current) => current.filter((id) => detected.some((adapter) => adapter.id === id)));
    setReviewer((current) =>
      current.length > 0 && detected.some((adapter) => adapter.id === current) ? current : "",
    );
  }, [detected]);

  // Un agent ne peut tenir qu'un rôle : le noyau refuse les doublons.
  useEffect(() => {
    setWorkers((current) => current.filter((id) => id !== lead));
    setReviewer((current) => (current === lead ? "" : current));
  }, [lead]);

  const form: TeamForm = {
    projectId,
    goal,
    lead,
    workers,
    reviewer: reviewer.length > 0 ? reviewer : null,
    workItems,
  };
  const build = buildTeamRequest(form);
  const errorKey = teamFieldErrorKey(build.error);
  const blockedHint = errorKey === null ? null : t(errorKey);
  const canLaunch =
    build.request !== null &&
    !controller.running &&
    !controller.requesting &&
    controller.approval === null;

  const toggleWorker = (id: string) => {
    if (workers.includes(id)) {
      setWorkers(workers.filter((entry) => entry !== id));
      return;
    }
    if (workers.length >= 3) {
      return;
    }
    setWorkers([...workers, id]);
    if (reviewer === id) {
      setReviewer("");
    }
  };

  const changeReviewer = (value: string) => {
    setReviewer(value);
    if (value.length > 0) {
      setWorkers((current) => current.filter((id) => id !== value));
    }
  };

  const state = controller.state;
  const runId = state.runId;
  const teamDiffs = filterTeamPendingDiffs(workspace.pendingDiffs, runId);
  const finished = state.status === "completed" || state.status === "failed";
  const showRun = state.status !== "idle" || state.tasks.length > 0;

  // La course se termine dans l'IPC asynchrone : on relit le registre des diffs.
  useEffect(() => {
    if (finished) {
      void workspace.refreshPendingDiffs();
    }
  }, [finished, runId, workspace.refreshPendingDiffs]);

  const openDiffPreview = async (id: string) => {
    setDiffPreviewBusy(true);
    setDiffPreviewError(null);
    try {
      setDiffPreview(await api.workspace.diffView(id));
    } catch (reason) {
      setDiffPreviewError(toAppError(reason));
    } finally {
      setDiffPreviewBusy(false);
    }
  };

  const showChanges = async () => {
    setReviewOpen(true);
    await workspace.refreshPendingDiffs();
  };

  const launch = async () => {
    if (!build.request) {
      return;
    }
    setReviewOpen(false);
    setDiffPreview(null);
    await controller.requestApproval(build.request);
  };

  const adapterOptions = (params: {
    disabled?: (adapter: AgentAdapterReport) => string | null;
    includeEmpty?: string;
  }) => (
    <>
      {params.includeEmpty ? <option value="">{params.includeEmpty}</option> : null}
      {adapters.map((adapter) => {
        const reason = !adapter.detected
          ? t("agent_mode.adapter_missing_hint", { diagnostic: adapter.diagnostic })
          : params.disabled?.(adapter) ?? null;
        return (
          <option
            key={adapter.id}
            value={adapter.id}
            disabled={reason !== null}
            title={reason ?? undefined}
          >
            {adapter.label}
            {adapter.detected ? "" : ` — ${t("agent_mode.missing")}`}
          </option>
        );
      })}
    </>
  );

  return (
    <div className="agent-panel-body team-panel">
      {adaptersError ? <ErrorBanner error={adaptersError} onRetry={onReloadAdapters} /> : null}
      {controller.launchError ? (
        <ErrorBanner error={controller.launchError} onDismiss={controller.dismissErrors} />
      ) : null}
      {controller.runError ? (
        <ErrorBanner error={controller.runError} onDismiss={controller.dismissErrors} />
      ) : null}
      {controller.refuseError ? (
        <ErrorBanner error={controller.refuseError} onDismiss={controller.dismissErrors} />
      ) : null}
      {controller.liveError ? <ErrorBanner error={controller.liveError} /> : null}

      <section className="agent-panel-section" aria-labelledby="team-compose-title">
        <div className="section-heading">
          <h3 id="team-compose-title">{t("team.compose_title")}</h3>
          {adaptersLoading ? <Spinner /> : null}
          {detected.length > 0 ? (
            <Badge tone="success">{t("team.detected_count", { count: detected.length })}</Badge>
          ) : (
            <Badge tone="danger">{t("team.no_adapter")}</Badge>
          )}
        </div>

        {!adaptersLoading && adapters.length === 0 ? (
          <EmptyState
            title={t("agent_mode.adapters_empty")}
            description={<p>{t("agent_mode.adapters_empty_hint")}</p>}
          />
        ) : null}

        <Field
          label={t("team.goal")}
          htmlFor="team-goal"
          hint={t("team.goal_hint")}
          required
          error={build.error === "goal" ? t("team.error.goal") : null}
        >
          <textarea
            id="team-goal"
            className="input textarea"
            rows={4}
            value={goal}
            placeholder={t("team.goal_placeholder")}
            disabled={controller.running}
            onChange={(event) => setGoal(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && canLaunch) {
                event.preventDefault();
                void launch();
              }
            }}
          />
        </Field>

        <div className="form-grid">
          <Field
            label={t("team.lead")}
            htmlFor="team-lead"
            hint={t("team.lead_hint")}
            required
            error={build.error === "lead" ? t("team.error.lead") : null}
          >
            <select
              id="team-lead"
              className="input"
              value={lead}
              disabled={controller.running}
              onChange={(event) => setLead(event.target.value)}
            >
              <option value="" disabled>
                {t("team.choose")}
              </option>
              {adapterOptions({})}
            </select>
          </Field>

          <Field
            label={t("team.reviewer")}
            htmlFor="team-reviewer"
            hint={t("team.reviewer_hint")}
            error={
              build.error === "reviewer_conflict" ? t("team.error.reviewer_conflict") : null
            }
          >
            <select
              id="team-reviewer"
              className="input"
              value={reviewer}
              disabled={controller.running}
              onChange={(event) => changeReviewer(event.target.value)}
            >
              {adapterOptions({
                includeEmpty: t("team.reviewer_auto"),
                disabled: (adapter) =>
                  adapter.id === lead
                    ? t("team.role_conflict", { role: t("team.role_lead") })
                    : workers.includes(adapter.id)
                      ? t("team.role_conflict", { role: t("team.role_worker") })
                      : null,
              })}
            </select>
          </Field>
        </div>

        <Field
          label={t("team.workers")}
          hint={t("team.workers_hint")}
          required
          error={
            build.error === "workers" ||
            build.error === "workers_duplicate" ||
            build.error === "worker_conflict"
              ? t(`team.error.${build.error}`)
              : null
          }
        >
          <ul className="agent-adapter-list" aria-label={t("team.workers")}>
            {adapters.map((adapter) => {
              const checked = workers.includes(adapter.id);
              const conflictRole: TeamRole | null =
                adapter.id === lead ? "lead" : adapter.id === reviewer ? "reviewer" : null;
              const reason = !adapter.detected
                ? t("agent_mode.adapter_missing_hint", { diagnostic: adapter.diagnostic })
                : conflictRole
                  ? t("team.role_conflict", { role: t(teamRoleLabelKey(conflictRole)) })
                  : null;
              const disabled = reason !== null || (!checked && workers.length >= 3);
              const missing = !adapter.detected;
              return (
                <li key={adapter.id}>
                  <label
                    className={
                      checked
                        ? "agent-option agent-adapter team-member-option selected"
                        : "agent-option agent-adapter team-member-option"
                    }
                    title={reason ?? adapter.label}
                  >
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={disabled || controller.running}
                      onChange={() => toggleWorker(adapter.id)}
                    />
                    <span className="agent-option-main">
                      <span className="agent-option-title">
                        <Users size={13} aria-hidden="true" />
                        {adapter.label}
                        <span className="small muted">{adapter.kind}</span>
                      </span>
                      <span className={missing ? "small danger-text" : "small muted"}>
                        {adapter.diagnostic}
                      </span>
                    </span>
                    <span className="inline-status">
                      {checked ? <Badge tone="accent">{t("agent_mode.selected")}</Badge> : null}
                      <Badge tone={missing ? "danger" : "success"}>
                        {missing ? t("agent_mode.missing") : t("agent_mode.detected")}
                      </Badge>
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        </Field>

        <Field
          label={t("team.work_items")}
          htmlFor="team-work-items"
          hint={t("team.work_items_hint")}
          error={build.error === "work_items" ? t("team.error.work_items") : null}
        >
          <select
            id="team-work-items"
            className="input compact"
            value={workItems}
            disabled={controller.running}
            onChange={(event) => setWorkItems(Number(event.target.value))}
          >
            {[1, 2, 3, 4].map((count) => (
              <option key={count} value={count}>
                {count}
              </option>
            ))}
          </select>
        </Field>

        <div className="button-row">
          <button
            type="button"
            className="primary"
            onClick={() => void launch()}
            disabled={!canLaunch}
            aria-busy={controller.requesting}
            title={blockedHint ?? t("team.launch_hint")}
          >
            {controller.requesting ? <Spinner /> : <Play size={14} aria-hidden="true" />}
            {controller.requesting ? t("team.requesting") : t("team.launch")}
          </button>
          {state.status === "idle" ? null : (
            <button
              type="button"
              className="ghost"
              onClick={controller.reset}
              disabled={controller.running}
            >
              <RefreshCw size={14} aria-hidden="true" />
              {t("team.clear")}
            </button>
          )}
        </div>
        <p className="muted small">{blockedHint ?? t("team.launch_hint")}</p>
      </section>

      {showRun ? (
        <section className="agent-panel-section" aria-labelledby="team-run-title">
          <div className="section-heading">
            <h3 id="team-run-title">{t("team.tasks")}</h3>
            <div className="chat-badges">
              <Badge tone={runTone(state.status)}>
                {state.status === "running" ? <Spinner /> : null}
                {t(teamStatusLabelKey(state.status))}
              </Badge>
              {runId ? (
                <span className="mono small" title={runId}>
                  {shortHash(runId, 18)}
                </span>
              ) : null}
              <span className="small muted">
                {t("team.task_count", { count: state.tasks.length })}
              </span>
            </div>
          </div>

          {state.tasks.length === 0 ? (
            <p className="small muted">{t("team.no_tasks")}</p>
          ) : (
            <ul className="agent-adapter-list team-task-list" aria-label={t("team.tasks")}>
              {state.tasks.map((task) => {
                const stateKey = teamTaskStateLabelKey(task.state);
                return (
                  <li key={task.taskId} className="agent-option team-task">
                    <span className="agent-option-main">
                      <span className="agent-option-title">
                        {task.role ? (
                          <Badge tone={roleTone(task.role)}>
                            {t(teamRoleLabelKey(task.role))}
                          </Badge>
                        ) : (
                          <Badge tone="neutral">
                            {task.rawRole || t("team.role_unknown")}
                          </Badge>
                        )}
                        <span className="team-task-title" title={task.title}>
                          {task.title}
                        </span>
                      </span>
                      {task.sessions > 0 ? (
                        <span className="small muted">
                          {t("team.task_sessions", { count: task.sessions })}
                        </span>
                      ) : null}
                    </span>
                    <span className="inline-status">
                      <Badge tone={taskStateTone(task.state)}>
                        {stateKey ? t(stateKey) : task.state}
                      </Badge>
                      {task.state === "running" ? <Spinner /> : null}
                    </span>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      ) : null}

      {finished ? (
        <section className="agent-panel-section" aria-labelledby="team-result-title">
          <div className="section-heading">
            <h3 id="team-result-title">
              {state.status === "completed" ? t("team.completed") : t("team.failed")}
            </h3>
            <Badge tone={runTone(state.status)}>{t(teamStatusLabelKey(state.status))}</Badge>
          </div>
          <h4>{t("team.summary")}</h4>
          <p className="small">{state.summary ?? t("agent_mode.no_summary")}</p>
          <h4>{t("team.files")}</h4>
          {state.files.length === 0 ? (
            <p className="small muted">{t("team.no_files")}</p>
          ) : (
            <ul className="plain-list">
              {state.files.map((path) => (
                <li key={path}>
                  <FileCode2 size={12} aria-hidden="true" />
                  <span className="mono small" title={path}>
                    {path}
                  </span>
                </li>
              ))}
            </ul>
          )}
          <div className="button-row">
            <button type="button" className="ghost" onClick={() => void showChanges()}>
              <FileDiff size={14} aria-hidden="true" />
              {t("team.view_changes")}
            </button>
          </div>
        </section>
      ) : null}

      {reviewOpen ? (
        <section className="agent-panel-section" aria-labelledby="team-review-title">
          <div className="section-heading">
            <h3 id="team-review-title">{t("team.review_title")}</h3>
            <div className="button-row compact">
              <Badge tone="warning">{t("team.review_count", { count: teamDiffs.length })}</Badge>
              <button
                type="button"
                className="ghost small"
                onClick={() => void workspace.refreshPendingDiffs()}
                disabled={workspace.pendingDiffBusy}
                aria-busy={workspace.pendingDiffBusy}
              >
                {workspace.pendingDiffBusy ? (
                  <Spinner />
                ) : (
                  <RefreshCw size={13} aria-hidden="true" />
                )}
                {t("common.refresh")}
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={t("common.close")}
                title={t("common.close")}
                onClick={() => setReviewOpen(false)}
              >
                <X size={14} />
              </button>
            </div>
          </div>
          <p className="small muted">{t("team.review_hint")}</p>
          {workspace.pendingDiffError ? (
            <ErrorBanner
              error={workspace.pendingDiffError}
              onRetry={() => void workspace.refreshPendingDiffs()}
            />
          ) : null}
          {diffPreview ? (
            <DiffView
              preview={diffPreview}
              busy={workspace.pendingDiffBusy}
              error={diffPreviewError}
              onAccept={() => {
                void workspace.acceptPendingDiff(diffPreview.id).then(() => setDiffPreview(null));
              }}
              onReject={() => {
                void workspace.rejectPendingDiff(diffPreview.id).then(() => setDiffPreview(null));
              }}
              onRetryPrepare={() => void openDiffPreview(diffPreview.id)}
              onClose={() => setDiffPreview(null)}
              errorIsConflict={false}
            />
          ) : null}
          {teamDiffs.length === 0 ? (
            <p className="small muted">{t("team.review_empty")}</p>
          ) : (
            <ul className="pending-diff-list">
              {teamDiffs.map((entry) => (
                <li key={entry.id} className="pending-diff-row">
                  <FileDiff size={13} aria-hidden="true" />
                  <span className="mono small pending-diff-path" title={entry.path}>
                    {entry.path}
                  </span>
                  {entry.delete ? (
                    <Badge tone="danger">{t("agent_mode.delete_file")}</Badge>
                  ) : null}
                  <div className="button-row compact">
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void openDiffPreview(entry.id)}
                      disabled={diffPreviewBusy}
                    >
                      <FileDiff size={12} aria-hidden="true" />
                      {t("workspace.view_diff")}
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void workspace.acceptPendingDiff(entry.id)}
                      disabled={workspace.pendingDiffBusy}
                    >
                      <Check size={12} aria-hidden="true" />
                      {t("workspace.diff_accept")}
                    </button>
                    <button
                      type="button"
                      className="danger small"
                      onClick={() => void workspace.rejectPendingDiff(entry.id)}
                      disabled={workspace.pendingDiffBusy}
                    >
                      <X size={12} aria-hidden="true" />
                      {t("workspace.diff_reject")}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : null}

      <Modal
        open={controller.approval !== null}
        title={t("team.approval_title")}
        size="wide"
        onClose={() => {
          if (!controller.refusing) {
            controller.closeApproval();
          }
        }}
        footer={
          <>
            <button
              type="button"
              className="ghost"
              onClick={controller.closeApproval}
              disabled={controller.refusing}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="danger"
              onClick={() => void controller.refuse()}
              disabled={controller.refusing}
              aria-busy={controller.refusing}
            >
              {controller.refusing ? <Spinner /> : <X size={14} aria-hidden="true" />}
              {t("team.refuse")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void controller.approve()}
              disabled={controller.refusing}
            >
              <ShieldAlert size={14} aria-hidden="true" />
              {t("team.approve")}
            </button>
          </>
        }
      >
        {controller.refuseError ? (
          <ErrorBanner error={controller.refuseError} onDismiss={controller.dismissErrors} />
        ) : null}
        {controller.approval ? (
          <>
            <dl className="detail-grid">
              <div>
                <dt>{t("team.approval_fingerprint")}</dt>
                <dd className="mono prewrap">{controller.approval.fingerprint}</dd>
              </div>
            </dl>
            <h4>{t("team.approval_plan")}</h4>
            <pre className="team-plan small">{controller.approval.plan_summary}</pre>
            <h4>{t("agent_mode.approval_detail")}</h4>
            <p className="small prewrap muted">{controller.approval.detail}</p>
            <p className="small muted">{t("team.approval_cancel_hint")}</p>
          </>
        ) : null}
      </Modal>
    </div>
  );
}
