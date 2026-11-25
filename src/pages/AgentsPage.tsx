import { useEffect, useState } from "react";
import { Bot, FolderOpen, RefreshCw } from "lucide-react";
import { api, type AgentInfo, type SessionRow } from "../lib/api";
import { securityLabelKey } from "../lib/agent";
import { formatDate, shortHash } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge, agentStateTone } from "../components/Badge";
import { EmptyState, LoadingRow } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

export function AgentsPage() {
  const { t, locale, project, navigate } = useApp();
  const adapters = useAsync(() => api.agents.list(), []);
  const sessions = useAsync<SessionRow[]>(async () => {
    if (!project) {
      return [];
    }
    return api.agents.sessions(project.id);
  }, [project?.id]);
  const [selectedAgentId, setSelectedAgentId] = useState("");

  const selected: AgentInfo | null =
    adapters.data?.find((adapter) => adapter.id === selectedAgentId) ?? null;

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
                      <Badge
                        tone={
                          adapter.sandbox?.sandboxed === false
                            ? "warning"
                            : adapter.sandbox?.engine_ready
                              ? "success"
                              : "danger"
                        }
                        title={
                          adapter.sandbox?.engine
                            ? t("agent_security.sandbox", { engine: adapter.sandbox.engine })
                            : undefined
                        }
                      >
                        {t(securityLabelKey(adapter.security))}
                      </Badge>
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
          </div>
          <Notice tone="warning" title={t("agents.launch_via_agent_mode")}>
            <p>{t("agents.launch_via_agent_mode_hint")}</p>
            <button type="button" className="ghost small" onClick={() => navigate("chat")}>
              {t("agents.open_agent_mode")}
            </button>
          </Notice>
          {selected ? (
            <p className="small muted">
              {t("agents.selected_agent", { label: selected.label })}
              {" · "}
              {t(securityLabelKey(selected.security))}
            </p>
          ) : null}
        </section>
      </div>

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
