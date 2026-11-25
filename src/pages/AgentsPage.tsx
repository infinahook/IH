import { useEffect, useRef, useState } from "react";
import { Ban, Bot, Eraser, FolderOpen, Play, RefreshCw } from "lucide-react";
import { api, type AgentInfo, type AgentState, type SessionRow } from "../lib/api";
import { formatDate, shortHash } from "../lib/format";
import { useAsync, useAction } from "../lib/hooks";
import { useTauriEvent } from "../lib/events";
import { useApp } from "../lib/store";
import { Badge, agentStateTone } from "../components/Badge";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

interface AgentOutputEvent {
  session_id: string;
  stream: "stdout" | "stderr";
  seq: number;
  data: string;
}

interface AgentStateEvent {
  session_id: string;
  state: AgentState;
}

interface OutputLine {
  id: number;
  stream: "stdout" | "stderr";
  text: string;
  partial: boolean;
}

const MAX_OUTPUT_LINES = 2_000;
const ACTIVE_STATES: AgentState[] = ["starting", "ready", "running", "waiting_approval"];

export function AgentsPage() {
  const { t, locale, project, navigate } = useApp();
  const adapters = useAsync(() => api.agents.list(), []);
  const sessions = useAsync<SessionRow[]>(async () => {
    if (!project) {
      return [];
    }
    return api.agents.sessions(project.id);
  }, [project?.id]);
  const start = useAction();
  const cancel = useAction();
  const stateCheck = useAction();

  const [selectedAgentId, setSelectedAgentId] = useState("");
  const [prompt, setPrompt] = useState("");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [sessionState, setSessionState] = useState<AgentState | null>(null);
  const [lines, setLines] = useState<OutputLine[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [stateMissing, setStateMissing] = useState(false);
  const pendingRef = useRef<Record<"stdout" | "stderr", string>>({ stdout: "", stderr: "" });
  const lineIdRef = useRef(0);
  const outputRef = useRef<HTMLPreElement>(null);

  const selected: AgentInfo | null =
    adapters.data?.find((adapter) => adapter.id === selectedAgentId) ?? null;
  const active = sessionState !== null && ACTIVE_STATES.includes(sessionState);

  useEffect(() => {
    if (!adapters.data) {
      return;
    }
    const current = adapters.data.find((adapter) => adapter.id === selectedAgentId);
    if (current) {
      return;
    }
    const preferred =
      adapters.data.find((adapter) => adapter.detected && adapter.enabled) ??
      adapters.data.find((adapter) => adapter.enabled) ??
      adapters.data[0];
    setSelectedAgentId(preferred?.id ?? "");
  }, [adapters.data, selectedAgentId]);

  useEffect(() => {
    setSessionId(null);
    setSessionState(null);
    setLines([]);
    pendingRef.current = { stdout: "", stderr: "" };
    lineIdRef.current = 0;
  }, [project?.id]);

  useEffect(() => {
    const element = outputRef.current;
    if (element) {
      element.scrollTop = element.scrollHeight;
    }
  }, [lines]);

  const appendOutput = (stream: "stdout" | "stderr", data: string) => {
    const combined = pendingRef.current[stream] + data;
    const parts = combined.split("\n");
    const tail = parts.pop() ?? "";
    pendingRef.current[stream] = tail;
    setLines((previous) => {
      const withoutPartial = previous.filter(
        (line) => !(line.partial && line.stream === stream),
      );
      const additions: OutputLine[] = parts.map((text) => ({
        id: (lineIdRef.current += 1),
        stream,
        text,
        partial: false,
      }));
      if (tail.length > 0) {
        additions.push({
          id: (lineIdRef.current += 1),
          stream,
          text: tail,
          partial: true,
        });
      }
      return [...withoutPartial, ...additions].slice(-MAX_OUTPUT_LINES);
    });
  };

  const flushPending = () => {
    const additions: OutputLine[] = [];
    for (const stream of ["stdout", "stderr"] as const) {
      const tail = pendingRef.current[stream];
      if (tail.length > 0) {
        additions.push({
          id: (lineIdRef.current += 1),
          stream,
          text: tail,
          partial: false,
        });
        pendingRef.current[stream] = "";
      }
    }
    if (additions.length > 0) {
      setLines((previous) => [...previous, ...additions].slice(-MAX_OUTPUT_LINES));
    }
  };

  const outputError = useTauriEvent<AgentOutputEvent>(
    "ih://agent-output",
    (payload) => {
      if (payload.session_id !== sessionId) {
        return;
      }
      const stream = payload.stream === "stderr" ? "stderr" : "stdout";
      appendOutput(stream, payload.data);
    },
    sessionId !== null,
  );

  const stateError = useTauriEvent<AgentStateEvent>(
    "ih://agent-state",
    (payload) => {
      if (payload.session_id !== sessionId) {
        return;
      }
      setSessionState(payload.state);
      if (["completed", "failed", "cancelled"].includes(payload.state)) {
        flushPending();
        sessions.reload();
      }
    },
    sessionId !== null,
  );

  const canStart =
    project !== null &&
    selected !== null &&
    selected.detected &&
    selected.enabled &&
    prompt.trim().length > 0 &&
    !active &&
    !start.busy;

  const handleStart = async () => {
    if (!project || !selected) {
      return;
    }
    setMessage(null);
    setLines([]);
    pendingRef.current = { stdout: "", stderr: "" };
    lineIdRef.current = 0;
    const started = await start.run(() =>
      api.agents.start(project.id, selected.id, prompt.trim()),
    );
    if (started) {
      setSessionId(started);
      setSessionState("starting");
      setMessage(t("agents.session_started", { id: started }));
      sessions.reload();
    }
  };

  const handleCancel = async () => {
    if (!sessionId) {
      return;
    }
    const done = await cancel.run(() => api.agents.cancel(sessionId));
    if (done !== undefined) {
      setMessage(t("agents.session_cancelled"));
      sessions.reload();
    }
  };

  return (
    <>
      <PageHeader
        title={t("agents.title")}
        subtitle={t("agents.start_hint")}
        actions={
          <button type="button" className="ghost" onClick={() => adapters.reload()}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        }
      />

      {adapters.error ? <ErrorBanner error={adapters.error} onRetry={adapters.reload} /> : null}
      {sessions.error ? <ErrorBanner error={sessions.error} onRetry={sessions.reload} /> : null}
      {start.error ? <ErrorBanner error={start.error} /> : null}
      {cancel.error ? <ErrorBanner error={cancel.error} /> : null}
      {outputError ? <ErrorBanner error={outputError} /> : null}
      {stateError ? <ErrorBanner error={stateError} /> : null}
      {stateCheck.error ? <ErrorBanner error={stateCheck.error} /> : null}
      {stateMissing ? <Notice tone="warning">{t("agents.session_unknown")}</Notice> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}
      {!project ? (
        <Notice tone="warning" title={t("gate.title")}>
          <p>{t("agents.needs_project")}</p>
          <button type="button" className="ghost small" onClick={() => navigate("projects")}>
            <FolderOpen size={14} aria-hidden="true" />
            {t("gate.action")}
          </button>
        </Notice>
      ) : null}

      <div className="agent-layout">
        <section className="card" aria-labelledby="agents-adapters">
          <div className="section-heading">
            <h2 id="agents-adapters">{t("agents.adapters")}</h2>
          </div>
          {adapters.loading ? (
            <LoadingRow label={t("common.loading")} />
          ) : adapters.data && adapters.data.length > 0 ? (
            <div className="agent-list" role="radiogroup" aria-labelledby="agents-adapters">
              {adapters.data.map((adapter) => {
                const checked = adapter.id === selectedAgentId;
                return (
                  <label
                    key={adapter.id}
                    className={checked ? "agent-option selected" : "agent-option"}
                  >
                    <input
                      type="radio"
                      name="agent-adapter"
                      value={adapter.id}
                      checked={checked}
                      onChange={() => setSelectedAgentId(adapter.id)}
                    />
                    <span className="agent-option-main">
                      <span className="agent-option-title">
                        <Bot size={15} aria-hidden="true" />
                        {adapter.label}
                      </span>
                      <span className="mono small">{adapter.program}</span>
                      {adapter.version ? (
                        <span className="muted small">
                          {t("agents.version_resolved", { version: adapter.version })}
                        </span>
                      ) : null}
                      {adapter.resolved_path ? (
                        <span className="muted small">
                          {t("agents.path_resolved", { path: adapter.resolved_path })}
                        </span>
                      ) : null}
                      {adapter.error ? (
                        <span className="small warning-text">{adapter.error}</span>
                      ) : null}
                    </span>
                    <span className="inline-status">
                      {!adapter.enabled ? <Badge>{t("common.disabled")}</Badge> : null}
                      <Badge tone={adapter.detected ? "success" : "danger"}>
                        {adapter.detected ? t("agents.detected") : t("agents.missing")}
                      </Badge>
                    </span>
                  </label>
                );
              })}
            </div>
          ) : (
            <EmptyState title={t("agents.no_agents")} description={<p>{t("agents.not_detected_hint")}</p>} />
          )}
        </section>

        <section className="card" aria-labelledby="agents-start">
          <div className="section-heading">
            <h2 id="agents-start">{t("agents.new_session")}</h2>
            {sessionState ? (
              <Badge tone={agentStateTone(sessionState)}>{t(`status.${sessionState}`)}</Badge>
            ) : null}
          </div>
          {selected && !selected.enabled ? (
            <Notice tone="warning" title={t("common.disabled")}>
              <p>{t("agents.adapter_disabled")}</p>
            </Notice>
          ) : null}
          {selected && !selected.detected ? (
            <Notice tone="warning" title={t("agents.missing")}>
              <p>{t("agents.needs_detected")}</p>
              <p className="muted small">{t("agents.not_detected_hint")}</p>
            </Notice>
          ) : null}
          <Field
            label={t("agents.prompt")}
            htmlFor="agent-prompt"
            hint={t("agents.prompt_hint")}
            required
          >
            <textarea
              id="agent-prompt"
              className="input textarea"
              rows={6}
              value={prompt}
              placeholder={t("agents.prompt_placeholder")}
              onChange={(event) => setPrompt(event.target.value)}
              disabled={active}
            />
          </Field>
          <div className="button-row">
            <button
              type="button"
              className="primary"
              onClick={() => void handleStart()}
              disabled={!canStart}
              aria-busy={start.busy}
              title={
                !project
                  ? t("agents.needs_project")
                  : selected && !selected.enabled
                    ? t("agents.adapter_disabled")
                    : selected && !selected.detected
                      ? t("agents.needs_detected")
                    : prompt.trim().length === 0
                      ? t("agents.instruction_required")
                      : active
                        ? t("agents.session_active_hint")
                        : undefined
              }
            >
              {start.busy ? <Spinner /> : <Play size={14} aria-hidden="true" />}
              {t("agents.start")}
            </button>
            <button
              type="button"
              className="ghost danger-text"
              onClick={() => void handleCancel()}
              disabled={!sessionId || !active || cancel.busy}
              aria-busy={cancel.busy}
            >
              {cancel.busy ? <Spinner /> : <Ban size={14} aria-hidden="true" />}
              {t("agents.cancel")}
            </button>
          </div>
          <p className="muted small">{t("agents.cancel_hint")}</p>
        </section>
      </div>

      <section className="card output-card" aria-labelledby="agents-output">
        <div className="section-heading">
          <h2 id="agents-output">{t("agents.output")}</h2>
          <div className="button-row compact">
            {sessionId ? (
              <span className="mono small" title={sessionId}>
                {t("agents.session")} {shortHash(sessionId, 14)}
              </span>
            ) : null}
            <button
              type="button"
              className="ghost small"
              onClick={async () => {
                if (!sessionId) {
                  return;
                }
                setStateMissing(false);
                const snapshot = await stateCheck.run(() => api.agents.state(sessionId));
                if (snapshot) {
                  setSessionState(snapshot.state);
                } else if (snapshot === null) {
                  setStateMissing(true);
                }
              }}
              disabled={!sessionId || stateCheck.busy}
              aria-busy={stateCheck.busy}
              aria-label={t("agents.refresh_state")}
              title={t("agents.refresh_state")}
            >
              {stateCheck.busy ? <Spinner /> : <RefreshCw size={13} aria-hidden="true" />}
              {t("agents.refresh_state")}
            </button>
            <button
              type="button"
              className="ghost small"
              onClick={() => {
                setLines([]);
                pendingRef.current = { stdout: "", stderr: "" };
              }}
              disabled={lines.length === 0}
            >
              <Eraser size={13} aria-hidden="true" />
              {t("agents.clear_output")}
            </button>
            <span className="stream-legend">
              <span className="stream-dot stdout" aria-hidden="true" />
              {t("agents.stream_output")}
            </span>
            <span className="stream-legend">
              <span className="stream-dot stderr" aria-hidden="true" />
              {t("agents.stream_error")}
            </span>
          </div>
        </div>
        <pre
          className="agent-output"
          role="log"
          aria-label={t("agents.output")}
          aria-live="off"
          ref={outputRef}
        >
          {lines.length === 0 ? (
            <span className="muted">{t("agents.output_empty")}</span>
          ) : (
            lines.map((line) => (
              <span key={line.id} className={`out-${line.stream}${line.partial ? " partial" : ""}`}>
                {line.text}
                {line.partial ? "" : "\n"}
              </span>
            ))
          )}
        </pre>
        {lines.length >= MAX_OUTPUT_LINES ? (
          <p className="muted small">{t("agents.truncation_notice")}</p>
        ) : null}
      </section>

      <section className="card" aria-labelledby="agents-sessions">
        <div className="section-heading">
          <h2 id="agents-sessions">{t("agents.recent_sessions")}</h2>
          <button
            type="button"
            className="ghost small"
            onClick={() => sessions.reload()}
            disabled={!project}
          >
            <RefreshCw size={13} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        </div>
        {!project ? (
          <p className="muted">{t("agents.needs_project")}</p>
        ) : sessions.loading ? (
          <LoadingRow label={t("common.loading")} />
        ) : sessions.data && sessions.data.length > 0 ? (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("agents.session")}</th>
                  <th scope="col">{t("agents.agent_label")}</th>
                  <th scope="col">{t("common.status")}</th>
                  <th scope="col">{t("agents.exit_code")}</th>
                  <th scope="col">{t("history.thread_created_at")}</th>
                </tr>
              </thead>
              <tbody>
                {sessions.data.map((session) => (
                  <tr key={session.id}>
                    <td className="mono" title={session.id}>
                      {shortHash(session.id, 12)}
                    </td>
                    <td>{session.agent_id}</td>
                    <td>
                      <Badge tone={agentStateTone(session.state)}>
                        {t(`status.${session.state}`)}
                      </Badge>
                    </td>
                    <td>{session.exit_code ?? "—"}</td>
                    <td>{formatDate(session.created_at, locale)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <EmptyState
            title={t("agents.no_sessions")}
            description={<p>{t("agents.no_sessions_hint")}</p>}
          />
        )}
      </section>
    </>
  );
}
