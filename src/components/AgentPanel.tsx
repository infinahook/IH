import { useEffect, useRef, useState } from "react";
import {
  Ban,
  Bot,
  Check,
  FileCode2,
  FileDiff,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldAlert,
  Terminal,
  X,
} from "lucide-react";
import { api, toAppError, type AppError, type AgentSessionDetail, type DiffPreview } from "../lib/api";
import {
  agentEventKey,
  agentEventLabelKey,
  agentPhaseLabelKey,
  filterAgentPendingDiffs,
  isAdapterSelectable,
  isTerminalAgentPhase,
  readOutputPayload,
  readStatePayload,
  type AgentPhase,
} from "../lib/agent";
import { formatRelativeTime } from "../lib/chat";
import { DiffView } from "./DiffView";
import { formatBytes, formatDate, shortHash } from "../lib/format";
import { useApp } from "../lib/store";
import type { AgentSessionController } from "../lib/useAgentSession";
import type { WorkspaceController } from "../lib/useWorkspace";
import { Badge, agentStateTone, riskTone, type BadgeTone } from "./Badge";
import { EmptyState, LoadingRow, Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";
import { Field } from "./Field";
import { Modal } from "./Modal";
import { Notice } from "./Notice";

function phaseTone(phase: AgentPhase): BadgeTone {
  switch (phase) {
    case "running":
    case "starting":
      return "accent";
    case "waiting_approval":
    case "approving":
      return "warning";
    case "completed":
      return "success";
    case "failed":
      return "danger";
    default:
      return "neutral";
  }
}

function sessionPhaseLabel(
  t: (key: string, vars?: Record<string, string | number>) => string,
  state: AgentSessionDetail,
): string {
  const key = `status.${state.session.state}`;
  const translated = t(key);
  return translated === key ? state.session.state : translated;
}

/**
 * Panneau du mode Agent : adaptateurs réellement détectés, lancement sous
 * approbation, sortie en flux bornée, sessions persistées et revue des
 * modifications produites par la session.
 *
 * Aucun contenu de sortie n'est interprété comme du HTML : tout est rendu en
 * texte brut via React (`pre` + `span`), jamais via `dangerouslySetInnerHTML`.
 */
export function AgentPanel({
  projectId,
  controller,
  workspace,
}: {
  projectId: string;
  controller: AgentSessionController;
  workspace: WorkspaceController;
}) {
  const { t, locale } = useApp();
  const c = controller;
  const [reviewOpen, setReviewOpen] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const outputRef = useRef<HTMLPreElement>(null);

  const phase = c.phase;
  const finished = isTerminalAgentPhase(phase);
  const agentDiffs = filterAgentPendingDiffs(workspace.pendingDiffs, c.sessionId);
  // Aperçu ligne à ligne d'un diff en attente (y compris ceux produits par
  // l'agent, qui ne passent pas par l'éditeur).
  const [diffPreview, setDiffPreview] = useState<DiffPreview | null>(null);
  const [diffPreviewError, setDiffPreviewError] = useState<AppError | null>(null);
  const [diffPreviewBusy, setDiffPreviewBusy] = useState(false);

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

  const outputEvents = c.events.filter((event) => event.kind === "output");
  const otherEvents = c.events.filter((event) => event.kind !== "output");
  const showSession = c.sessionId !== null || c.detail !== null;

  useEffect(() => {
    void workspace.refreshPendingDiffs();
    // Les diffs en attente sont un registre global : on le relit à l'ouverture.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  useEffect(() => {
    if (finished) {
      setCancelRequested(false);
    }
  }, [finished]);

  useEffect(() => {
    const element = outputRef.current;
    if (element) {
      element.scrollTop = element.scrollHeight;
    }
  }, [c.events.length]);

  const blockedHint =
    c.launchBlocked === "adapter"
      ? t("agent_mode.launch_needs_adapter")
      : c.launchBlocked === "task"
        ? t("agent_mode.task_required")
        : c.launchBlocked === "active"
          ? t("agent_mode.launch_active")
          : c.launchBlocked === "busy"
            ? t("agent_mode.launch_busy")
            : t("agent_mode.launch_hint");

  const phaseLabel =
    cancelRequested && c.active ? t("agent_mode.cancelling") : t(agentPhaseLabelKey(phase));

  const handleCancel = async () => {
    const accepted = await c.cancelSession();
    setCancelRequested(accepted);
  };

  const showChanges = async () => {
    setReviewOpen(true);
    await workspace.refreshPendingDiffs();
  };

  return (
    <section
      className="card chat-conversation agent-conversation"
      aria-labelledby="agent-panel-title"
    >
      <header className="chat-conversation-head">
        <div className="chat-conversation-heading">
          <h2 id="agent-panel-title">{t("agent_mode.title")}</h2>
          <div className="chat-badges">
            <Badge tone={phaseTone(phase)}>
              {c.active ? <Spinner /> : null}
              {phaseLabel}
            </Badge>
            {c.sessionId ? (
              <span className="mono small" title={c.sessionId}>
                {shortHash(c.sessionId, 14)}
              </span>
            ) : null}
            {c.selectedAdapter ? (
              <Badge tone="info">{c.selectedAdapter.label}</Badge>
            ) : (
              <Badge tone="danger">{t("agent_mode.no_adapter")}</Badge>
            )}
            {c.steps > 0 ? (
              <span className="small muted">{t("agent_mode.steps", { count: c.steps })}</span>
            ) : null}
          </div>
        </div>
        <div className="button-row compact">
          <button
            type="button"
            className="ghost small"
            onClick={() => {
              c.reloadAdapters();
              c.reloadSessions();
            }}
            disabled={c.adaptersLoading || c.sessionsLoading}
          >
            <RefreshCw size={13} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        </div>
      </header>

      <div className="agent-panel-body">
        {c.adaptersError ? (
          <ErrorBanner error={c.adaptersError} onRetry={c.reloadAdapters} />
        ) : null}
        {c.sessionsError ? (
          <ErrorBanner error={c.sessionsError} onRetry={c.reloadSessions} />
        ) : null}
        {c.approval === null && c.launchError ? (
          <ErrorBanner error={c.launchError} onDismiss={c.dismissErrors} />
        ) : null}
        {c.cancelError ? <ErrorBanner error={c.cancelError} onDismiss={c.dismissErrors} /> : null}
        {c.liveError ? <ErrorBanner error={c.liveError} /> : null}
        {c.notice ? (
          <div className="workspace-notice-row">
            <Notice tone={c.notice.tone}>{c.notice.text}</Notice>
            <button
              type="button"
              className="icon-button"
              aria-label={t("common.close")}
              onClick={c.dismissNotice}
            >
              <X size={14} />
            </button>
          </div>
        ) : null}

        <section className="agent-panel-section" aria-labelledby="agent-adapters-title">
          <div className="section-heading">
            <h3 id="agent-adapters-title">{t("agent_mode.adapters")}</h3>
            {c.adaptersLoading ? <Spinner /> : null}
          </div>
          {!c.adaptersLoading && c.adapters.length === 0 ? (
            <EmptyState
              title={t("agent_mode.adapters_empty")}
              description={<p>{t("agent_mode.adapters_empty_hint")}</p>}
            />
          ) : (
            <ul className="agent-adapter-list" aria-label={t("agent_mode.adapters")}>
              {c.adapters.map((adapter) => {
                const selectable = isAdapterSelectable(adapter);
                const selected = adapter.id === c.selectedAgentId;
                return (
                  <li key={adapter.id}>
                    <button
                      type="button"
                      className={
                        selected
                          ? "agent-option agent-adapter selected"
                          : "agent-option agent-adapter"
                      }
                      aria-pressed={selected}
                      disabled={!selectable}
                      title={
                        selectable
                          ? adapter.label
                          : t("agent_mode.adapter_missing_hint", {
                              diagnostic: adapter.diagnostic,
                            })
                      }
                      onClick={() => c.selectAdapter(adapter.id)}
                    >
                      <span className="agent-option-main">
                        <span className="agent-option-title">
                          <Bot size={14} aria-hidden="true" />
                          {adapter.label}
                          <span className="small muted">{adapter.kind}</span>
                        </span>
                        <span className="mono small">
                          {adapter.program}
                          {adapter.version ? ` · ${adapter.version}` : ` · ${t("agent_mode.version_unknown")}`}
                        </span>
                        <span className="small muted">
                          {adapter.path
                            ? t("agent_mode.path", { path: adapter.path })
                            : t("agent_mode.path_unknown")}
                        </span>
                        <span className={selectable ? "small muted" : "small danger-text"}>
                          {adapter.diagnostic}
                        </span>
                      </span>
                      <span className="inline-status">
                        {selected ? (
                          <Badge tone="accent">{t("agent_mode.selected")}</Badge>
                        ) : null}
                        <Badge tone={selectable ? "success" : "danger"}>
                          {selectable ? t("agent_mode.detected") : t("agent_mode.missing")}
                        </Badge>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        <section className="agent-panel-section" aria-labelledby="agent-launch-title">
          <div className="section-heading">
            <h3 id="agent-launch-title">{t("agent_mode.launch_title")}</h3>
            {c.selectedAdapter ? (
              <Badge tone="success">{c.selectedAdapter.label}</Badge>
            ) : (
              <Badge tone="danger">{t("agent_mode.no_adapter")}</Badge>
            )}
          </div>
          <Field
            label={t("agent_mode.task")}
            htmlFor="agent-task"
            hint={t("agent_mode.task_hint")}
            required
          >
            <textarea
              id="agent-task"
              className="input textarea"
              rows={5}
              value={c.task}
              placeholder={t("agent_mode.task_placeholder")}
              disabled={c.active || c.launching}
              onChange={(event) => c.setTask(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && c.canLaunch) {
                  event.preventDefault();
                  void c.requestApproval();
                }
              }}
            />
          </Field>
          <div className="button-row">
            <button
              type="button"
              className="primary"
              onClick={() => void c.requestApproval()}
              disabled={!c.canLaunch}
              aria-busy={c.launching}
              title={blockedHint}
            >
              {c.launching ? <Spinner /> : <Play size={14} aria-hidden="true" />}
              {c.launching ? t("agent_mode.requesting") : t("agent_mode.launch")}
            </button>
            {(c.active && c.sessionId !== null) || cancelRequested ? (
              <button
                type="button"
                className="danger"
                onClick={() => void handleCancel()}
                disabled={c.cancelling}
                aria-busy={c.cancelling}
              >
                {c.cancelling ? <Spinner /> : <Ban size={14} aria-hidden="true" />}
                {c.cancelling || cancelRequested
                  ? t("agent_mode.cancelling")
                  : t("agent_mode.cancel_session")}
              </button>
            ) : null}
          </div>
          <p className="muted small">{blockedHint}</p>
        </section>

        {showSession ? (
          <section className="agent-panel-section" aria-labelledby="agent-session-title">
            <div className="section-heading">
              <h3 id="agent-session-title">{t("agent_mode.session")}</h3>
              <div className="chat-badges">
                <Badge tone={phaseTone(phase)}>{phaseLabel}</Badge>
                <span className="small muted">
                  {t("agent_mode.steps", { count: c.steps })}
                </span>
                {c.outputBytes > 0 ? (
                  <span className="small muted">{formatBytes(c.outputBytes)}</span>
                ) : null}
                {c.bounded ? (
                  <Badge tone="warning">{t("agent_mode.bounded")}</Badge>
                ) : null}
              </div>
            </div>

            {c.detailError ? (
              <ErrorBanner
                error={c.detailError}
                onRetry={() => {
                  if (c.sessionId) {
                    void c.loadSession(c.sessionId);
                  }
                }}
              />
            ) : null}
            {c.detailLoading && c.detail === null ? (
              <LoadingRow label={t("agent_mode.session_loading")} />
            ) : null}
            {c.detail ? (
              <dl className="detail-grid">
                <div>
                  <dt>{t("agent_mode.session_adapter")}</dt>
                  <dd>{c.detail.adapter ?? t("agent_mode.unknown_adapter")}</dd>
                </div>
                <div>
                  <dt>{t("agent_mode.session_state")}</dt>
                  <dd>{sessionPhaseLabel(t, c.detail)}</dd>
                </div>
                <div>
                  <dt>{t("agent_mode.session_workdir")}</dt>
                  <dd className="mono">{c.detail.workdir ?? "—"}</dd>
                </div>
                <div>
                  <dt>{t("agent_mode.session_created")}</dt>
                  <dd>{formatDate(c.detail.session.created_at, locale)}</dd>
                </div>
                <div>
                  <dt>{t("agent_mode.session_ended")}</dt>
                  <dd>{formatDate(c.detail.session.ended_at, locale)}</dd>
                </div>
                <div>
                  <dt>{t("agent_mode.session_task")}</dt>
                  <dd>{c.detail.task ?? t("agent_mode.unknown_task")}</dd>
                </div>
              </dl>
            ) : null}

            {otherEvents.length > 0 ? (
              <ul className="agent-event-list" aria-label={t("agent_mode.events")}>
                {otherEvents.map((event) => {
                  const labelKey = agentEventLabelKey(event.kind);
                  const state = readStatePayload(event.payload);
                  return (
                    <li key={agentEventKey(event)}>
                      <span className="mono small">
                        {labelKey ? t(labelKey) : event.kind}
                      </span>
                      {state ? (
                        <span className="small muted">{t(agentPhaseLabelKey(state))}</span>
                      ) : null}
                      <span className="small muted">{event.seq}</span>
                    </li>
                  );
                })}
              </ul>
            ) : null}

            <div className="agent-output-head">
              <span className="inline-status">
                <Terminal size={13} aria-hidden="true" />
                <span className="small">{t("agent_mode.output")}</span>
              </span>
              <span className="stream-legend">
                <span className="stream-dot stdout" aria-hidden="true" />
                {t("agents.stream_output")}
              </span>
              <span className="stream-legend">
                <span className="stream-dot stderr" aria-hidden="true" />
                {t("agents.stream_error")}
              </span>
            </div>
            <pre
              className="agent-output"
              role="log"
              aria-label={t("agent_mode.output")}
              aria-live="off"
              ref={outputRef}
            >
              {outputEvents.length === 0 ? (
                <span className="muted">{t("agent_mode.output_empty")}</span>
              ) : (
                outputEvents.map((event) => {
                  const output = readOutputPayload(event.payload);
                  if (!output) {
                    return null;
                  }
                  return (
                    <span key={agentEventKey(event)} className={`out-${output.stream}`}>
                      {output.data}
                    </span>
                  );
                })
              )}
            </pre>
            {c.bounded ? (
              <p className="small muted">{t("agent_mode.output_truncated")}</p>
            ) : null}

            {finished ? (
              <div className="agent-result">
                {c.sessionError ? (
                  <Notice tone="error" title={t("agent_mode.session_error")}>
                    <p>{c.sessionError}</p>
                  </Notice>
                ) : null}
                <h4>{t("agent_mode.summary")}</h4>
                <p className="small">{c.summary ?? t("agent_mode.no_summary")}</p>
                <h4>{t("agent_mode.files")}</h4>
                {c.files.length === 0 ? (
                  <p className="small muted">{t("agent_mode.no_files")}</p>
                ) : (
                  <ul className="plain-list">
                    {c.files.map((path) => (
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
                    {t("agent_mode.view_changes")}
                  </button>
                  <button
                    type="button"
                    className="ghost"
                    onClick={() => void c.relaunch()}
                    disabled={
                      c.launching || c.selectedAdapter === null || c.task.trim().length === 0
                    }
                  >
                    {c.launching ? <Spinner /> : <RotateCcw size={14} aria-hidden="true" />}
                    {t("agent_mode.relaunch")}
                  </button>
                </div>
                <p className="small muted">{t("agent_mode.relaunch_hint")}</p>
              </div>
            ) : null}
          </section>
        ) : null}

        {reviewOpen ? (
          <section className="agent-panel-section" aria-labelledby="agent-review-title">
            <div className="section-heading">
              <h3 id="agent-review-title">{t("agent_mode.review_title")}</h3>
              <div className="button-row compact">
                <Badge tone="warning">
                  {t("agent_mode.review_count", { count: agentDiffs.length })}
                </Badge>
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
            <p className="small muted">{t("agent_mode.review_hint")}</p>
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
            {agentDiffs.length === 0 ? (
              <p className="small muted">{t("agent_mode.review_empty")}</p>
            ) : (
              <ul className="pending-diff-list">
                {agentDiffs.map((entry) => (
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

        <section className="agent-panel-section" aria-labelledby="agent-sessions-title">
          <div className="section-heading">
            <h3 id="agent-sessions-title">{t("agent_mode.sessions")}</h3>
            <span className="small muted">
              {c.sessionsLoading
                ? t("common.loading")
                : t("agent_mode.session_count", { count: c.sessions.length })}
            </span>
          </div>
          {c.sessions.length === 0 && !c.sessionsLoading ? (
            <EmptyState
              title={t("agent_mode.sessions_empty")}
              description={<p>{t("agent_mode.sessions_empty_hint")}</p>}
            />
          ) : (
            <ul className="agent-session-list" aria-label={t("agent_mode.sessions")}>
              {c.sessions.map((entry) => {
                const selected = entry.session.id === c.sessionId;
                return (
                  <li key={entry.session.id}>
                    <button
                      type="button"
                      className={
                        selected
                          ? "agent-option agent-session-row selected"
                          : "agent-option agent-session-row"
                      }
                      aria-pressed={selected}
                      onClick={() => void c.loadSession(entry.session.id)}
                    >
                      <span className="agent-option-main">
                        <span className="agent-option-title">
                          <Bot size={13} aria-hidden="true" />
                          {entry.adapter ?? t("agent_mode.unknown_adapter")}
                          <span className="small muted">
                            {formatRelativeTime(entry.session.created_at, locale)}
                          </span>
                        </span>
                        <span className="small muted agent-session-summary">
                          {entry.summary ?? entry.task ?? t("agent_mode.no_summary")}
                        </span>
                        <span className="small muted">
                          {t("agent_mode.session_files", { count: entry.files.length })}
                        </span>
                      </span>
                      <span className="inline-status">
                        <Badge tone={agentStateTone(entry.session.state)}>
                          {sessionPhaseLabel(t, entry)}
                        </Badge>
                        {entry.session.state === "running" ? <Spinner /> : null}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>

      <Modal
        open={c.approval !== null}
        title={t("agent_mode.approval_title")}
        size="wide"
        onClose={() => {
          if (!c.launching) {
            c.closeApproval();
          }
        }}
        footer={
          <>
            <button
              type="button"
              className="ghost"
              onClick={c.closeApproval}
              disabled={c.launching}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="danger"
              onClick={() => void c.refuse()}
              disabled={c.launching || c.refusing}
              aria-busy={c.refusing}
            >
              {c.refusing ? <Spinner /> : <X size={14} aria-hidden="true" />}
              {t("agent_mode.approval_refuse")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void c.approve()}
              disabled={c.launching}
              aria-busy={c.launching}
            >
              {c.launching ? <Spinner /> : <ShieldAlert size={14} aria-hidden="true" />}
              {t("agent_mode.approval_allow_once")}
            </button>
          </>
        }
      >
        {c.launchError ? (
          <ErrorBanner error={c.launchError} onDismiss={c.dismissErrors} />
        ) : null}
        {c.refuseError ? (
          <ErrorBanner error={c.refuseError} onDismiss={c.dismissErrors} />
        ) : null}
        {c.approval ? (
          <>
            <dl className="detail-grid">
              <div>
                <dt>{t("agent_mode.approval_command")}</dt>
                <dd className="mono prewrap">{c.approval.command}</dd>
              </div>
              <div>
                <dt>{t("agent_mode.approval_workdir")}</dt>
                <dd className="mono prewrap">{c.approval.workdir}</dd>
              </div>
              <div>
                <dt>{t("agent_mode.approval_risk")}</dt>
                <dd>
                  <Badge tone={riskTone(c.approval.risk)}>
                    {t(`agent_mode.risk_${c.approval.risk}`)}
                  </Badge>
                </dd>
              </div>
              <div>
                <dt>{t("agent_mode.approval_fingerprint")}</dt>
                <dd className="mono prewrap">{c.approval.fingerprint}</dd>
              </div>
            </dl>
            <h4>{t("agent_mode.approval_detail")}</h4>
            <p className="small prewrap muted">{c.approval.detail}</p>
            <p className="small muted">{t("agent_mode.approval_cancel_hint")}</p>
          </>
        ) : null}
      </Modal>
    </section>
  );
}
