import { useState } from "react";
import {
  Cable,
  Download,
  Pencil,
  Play,
  Plus,
  Power,
  RefreshCw,
  Trash2,
  Wrench,
} from "lucide-react";
import {
  AppError,
  api,
  type McpCallRecord,
  type McpRemoteServer,
  type McpRemoteTool,
} from "../lib/api";
import { riskTone, schemaFields, statusTone } from "../lib/mcp";
import { formatDate } from "../lib/format";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { McpCallPanel, McpSecurityHint } from "../components/McpCallPanel";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

const TRANSPORTS = ["streamable_http", "sse"] as const;

interface ServerForm {
  editingId: string | null;
  name: string;
  url: string;
  transport: string;
  timeout: string;
  headers: string;
  token: string;
  enabled: boolean;
  allowAutoApprove: boolean;
}

const EMPTY_FORM: ServerForm = {
  editingId: null,
  name: "",
  url: "",
  transport: "streamable_http",
  timeout: "30",
  headers: "{}",
  token: "",
  enabled: true,
  allowAutoApprove: false,
};

export function McpPage() {
  const { t, locale } = useApp();
  const servers = useAsync(() => api.mcpRemote.list(), []);
  const tools = useAsync(() => api.mcpRemote.tools(), []);
  const calls = useAsync(() => api.mcpRemote.calls(50), []);
  const saveAction = useAction();
  const serverAction = useAction();
  const toolAction = useAction();
  const callAction = useAction();
  const deleteAction = useAction();
  const [form, setForm] = useState<ServerForm>(EMPTY_FORM);
  const [formError, setFormError] = useState<AppError | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [messageTone, setMessageTone] = useState<"success" | "warning">("success");
  const [toDelete, setToDelete] = useState<McpRemoteServer | null>(null);
  const [busyServer, setBusyServer] = useState<string | null>(null);
  const [exported, setExported] = useState<string | null>(null);
  const [cliFlavor, setCliFlavor] = useState("claude");

  const serverList = servers.data ?? [];
  const allTools = tools.data ?? [];
  const toolsOf = (serverId: string): McpRemoteTool[] =>
    allTools.filter((tool) => tool.server_id === serverId);
  const notify = (text: string, tone: "success" | "warning" = "success") => {
    setMessageTone(tone);
    setMessage(text);
  };

  const resetForm = () => {
    setForm(EMPTY_FORM);
    setFormError(null);
  };

  const parseHeaders = (): Record<string, string> | null => {
    const raw = form.headers.trim();
    if (raw.length === 0) {
      return {};
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        setFormError(new AppError("invalid_input", t("mcp.headers_hint"), true));
        return null;
      }
      const headers: Record<string, string> = {};
      for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
        if (typeof value !== "string") {
          setFormError(new AppError("invalid_input", t("mcp.headers_hint"), true));
          return null;
        }
        headers[key] = value;
      }
      return headers;
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      setFormError(new AppError("invalid_input", t("mcp.invalid_json", { message: detail }), true));
      return null;
    }
  };

  const submitServer = async () => {
    setMessage(null);
    setFormError(null);
    if (form.name.trim().length === 0 || form.url.trim().length === 0) {
      setFormError(new AppError("invalid_input", t("mcp.name_url_required"), true));
      return;
    }
    const headers = parseHeaders();
    if (headers === null) {
      return;
    }
    const timeout = Number(form.timeout);
    if (!Number.isInteger(timeout) || timeout < 1) {
      setFormError(new AppError("invalid_input", t("mcp.timeout_hint"), true));
      return;
    }
    const saved = await saveAction.run(() =>
      api.mcpRemote.upsert({
        serverId: form.editingId ?? undefined,
        name: form.name.trim(),
        url: form.url.trim(),
        transport: form.transport,
        headers,
        timeoutSecs: timeout,
        enabled: form.enabled,
        allowAutoApprove: form.allowAutoApprove,
        token: form.token.trim().length > 0 ? form.token.trim() : undefined,
      }),
    );
    if (saved) {
      notify(t(form.editingId ? "mcp.saved" : "mcp.created"));
      resetForm();
      servers.reload();
      tools.reload();
    }
  };

  const startEdit = (server: McpRemoteServer) => {
    setMessage(null);
    setFormError(null);
    setForm({
      editingId: server.id,
      name: server.name,
      url: server.url,
      transport: server.transport,
      timeout: String(server.timeout_secs),
      headers: JSON.stringify({}, null, 2),
      token: "",
      enabled: server.enabled,
      allowAutoApprove: server.allow_auto_approve,
    });
  };

  const connect = async (server: McpRemoteServer) => {
    setBusyServer(server.id);
    const loaded = await serverAction.run(() => api.mcpRemote.connect(server.id));
    setBusyServer(null);
    if (loaded) {
      notify(t("mcp.connected_msg", { name: server.name }));
      servers.reload();
      tools.reload();
    }
  };

  const disconnect = async (server: McpRemoteServer) => {
    setBusyServer(server.id);
    const done = await serverAction.run(async () => {
      await api.mcpRemote.disconnect(server.id);
      return true;
    });
    setBusyServer(null);
    if (done) {
      notify(t("mcp.disconnected_msg", { name: server.name }));
      servers.reload();
    }
  };

  const testConnection = async (server: McpRemoteServer) => {
    setBusyServer(server.id);
    const outcome = await serverAction.run(() => api.mcpRemote.test(server.id));
    setBusyServer(null);
    if (outcome) {
      if (outcome.ok) {
        notify(t("mcp.tested", { count: outcome.tool_count }));
      } else {
        notify(t("mcp.test_failed", { message: outcome.error ?? "" }), "warning");
      }
      servers.reload();
    }
  };

  const refreshTools = async (server: McpRemoteServer) => {
    setBusyServer(server.id);
    const loaded = await serverAction.run(() => api.mcpRemote.refreshTools(server.id));
    setBusyServer(null);
    if (loaded) {
      notify(t("mcp.tools_refreshed", { count: loaded.length }));
      servers.reload();
      tools.reload();
    }
  };

  const toggleServer = async (server: McpRemoteServer) => {
    const updated = await serverAction.run(() =>
      api.mcpRemote.setEnabled(server.id, !server.enabled),
    );
    if (updated) {
      notify(
        t(updated.enabled ? "mcp.server_enabled" : "mcp.server_disabled", { name: updated.name }),
      );
      servers.reload();
    }
  };

  const confirmDelete = async () => {
    const server = toDelete;
    if (!server) {
      return;
    }
    const done = await deleteAction.run(async () => {
      await api.mcpRemote.remove(server.id);
      return true;
    });
    if (done) {
      setToDelete(null);
      setExported(null);
      notify(t("mcp.deleted"));
      servers.reload();
      tools.reload();
      calls.reload();
    }
  };

  const toggleTool = async (serverId: string, name: string, enabled: boolean) => {
    const updated = await toolAction.run(() =>
      api.mcpRemote.setToolEnabled(serverId, name, enabled),
    );
    if (updated) {
      tools.setData((current) =>
        (current ?? []).map((tool) =>
          tool.server_id === serverId && tool.name === name ? updated : tool,
        ),
      );
      notify(t(updated.enabled ? "mcp.tool_enabled" : "mcp.tool_disabled", { name }));
    }
  };

  const exportConfig = async () => {
    const config = await callAction.run(() => api.mcpRemote.exportConfig());
    if (config !== undefined && config !== null) {
      setExported(config);
      notify(t("mcp.export_done"));
    }
  };

  const exportAgentConfig = async () => {
    const config = await callAction.run(() => api.mcpRemote.agentConfig(cliFlavor));
    if (config !== undefined && config !== null) {
      setExported(config);
      notify(t("mcp.agent_config_done", { flavor: cliFlavor }));
    }
  };

  return (
    <>
      <PageHeader
        title={t("mcp.title")}
        subtitle={t("mcp.subtitle")}
        actions={
          <div className="button-row compact">
            <select
              className="input"
              aria-label={t("mcp.agent_flavor")}
              value={cliFlavor}
              onChange={(event) => setCliFlavor(event.target.value)}
            >
              <option value="claude">Claude Code</option>
              <option value="gemini">Gemini CLI</option>
              <option value="opencode">OpenCode</option>
            </select>
            <button type="button" className="ghost" onClick={() => void exportAgentConfig()}>
              <Download size={15} aria-hidden="true" />
              {t("mcp.agent_config")}
            </button>
            <button type="button" className="ghost" onClick={() => void exportConfig()}>
              <Download size={15} aria-hidden="true" />
              {t("mcp.export_config")}
            </button>
            <button type="button" className="ghost" onClick={() => servers.reload()}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
          </div>
        }
      />

      {servers.error ? <ErrorBanner error={servers.error} onRetry={servers.reload} /> : null}
      {tools.error ? <ErrorBanner error={tools.error} onRetry={tools.reload} /> : null}
      {saveAction.error ? <ErrorBanner error={saveAction.error} /> : null}
      {serverAction.error ? <ErrorBanner error={serverAction.error} /> : null}
      {toolAction.error ? <ErrorBanner error={toolAction.error} /> : null}
      {callAction.error ? <ErrorBanner error={callAction.error} /> : null}
      {formError ? <ErrorBanner error={formError} /> : null}
      {message ? <Notice tone={messageTone}>{message}</Notice> : null}

      <section className="card" aria-labelledby="mcp-form">
        <div className="section-heading">
          <h2 id="mcp-form">
            {form.editingId ? t("mcp.edit_server") : t("mcp.new_server")}
          </h2>
          <Cable size={17} aria-hidden="true" className="muted-icon" />
        </div>
        <div className="form-grid">
          <Field label={t("mcp.name")} htmlFor="mcp-name" required>
            <input
              id="mcp-name"
              className="input"
              value={form.name}
              placeholder={t("mcp.name_placeholder")}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
            />
          </Field>
          <Field label={t("mcp.transport")} htmlFor="mcp-transport">
            <select
              id="mcp-transport"
              className="input"
              value={form.transport}
              onChange={(event) => setForm({ ...form, transport: event.target.value })}
            >
              {TRANSPORTS.map((transport) => (
                <option key={transport} value={transport}>
                  {t(`mcp.transport_${transport}`)}
                </option>
              ))}
            </select>
          </Field>
        </div>
        <Field label={t("mcp.url")} htmlFor="mcp-url" hint={t("mcp.url_hint")} required>
          <input
            id="mcp-url"
            className="input mono"
            value={form.url}
            placeholder="https://…/mcp"
            onChange={(event) => setForm({ ...form, url: event.target.value })}
          />
        </Field>
        <div className="form-grid">
          <Field label={t("mcp.timeout")} htmlFor="mcp-timeout" hint={t("mcp.timeout_hint")}>
            <input
              id="mcp-timeout"
              className="input"
              type="number"
              min={1}
              value={form.timeout}
              onChange={(event) => setForm({ ...form, timeout: event.target.value })}
            />
          </Field>
          <Field label={t("mcp.token")} htmlFor="mcp-token" hint={t("mcp.token_hint")}>
            <input
              id="mcp-token"
              className="input"
              type="password"
              autoComplete="off"
              value={form.token}
              placeholder={form.editingId ? t("mcp.token_keep") : ""}
              onChange={(event) => setForm({ ...form, token: event.target.value })}
            />
          </Field>
        </div>
        <Field label={t("mcp.headers")} htmlFor="mcp-headers" hint={t("mcp.headers_hint")}>
          <textarea
            id="mcp-headers"
            className="input textarea mono"
            rows={3}
            spellCheck={false}
            value={form.headers}
            onChange={(event) => setForm({ ...form, headers: event.target.value })}
          />
        </Field>
        <div className="button-row">
          <label className="check">
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(event) => setForm({ ...form, enabled: event.target.checked })}
            />
            {t("mcp.enabled")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={form.allowAutoApprove}
              onChange={(event) => setForm({ ...form, allowAutoApprove: event.target.checked })}
            />
            {t("mcp.auto_approve")}
          </label>
          <span className="muted small">{t("mcp.auto_approve_hint")}</span>
        </div>
        <div className="button-row">
          {form.editingId ? (
            <button type="button" className="ghost small" onClick={resetForm}>
              {t("common.cancel")}
            </button>
          ) : null}
          <button
            type="button"
            className="primary"
            onClick={() => void submitServer()}
            disabled={saveAction.busy}
            aria-busy={saveAction.busy}
          >
            {saveAction.busy ? <Spinner /> : <Plus size={14} aria-hidden="true" />}
            {form.editingId ? t("common.save") : t("common.create")}
          </button>
        </div>
      </section>

      {servers.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : serverList.length === 0 ? (
        <EmptyState
          icon={<Cable size={22} aria-hidden="true" />}
          title={t("mcp.no_servers")}
          description={<p>{t("mcp.no_servers_hint")}</p>}
        />
      ) : (
        <div className="mcp-server-list">
          {serverList.map((server) => {
            const serverTools = toolsOf(server.id);
            const busy = busyServer === server.id || serverAction.busy;
            return (
              <article className="card mcp-server-card" key={server.id}>
                <header className="provider-header">
                  <div>
                    <div className="provider-title">
                      <h2>{server.name}</h2>
                      <Badge tone={statusTone(server.last_status, server.connected)}>
                        {server.connected
                          ? t("mcp.status_connected")
                          : t(`mcp.status_${server.last_status}`, {
                              defaultValue: server.last_status,
                            })}
                      </Badge>
                      <Badge tone="neutral">{t(`mcp.transport_${server.transport}`)}</Badge>
                      {!server.enabled ? <Badge tone="neutral">{t("mcp.disabled_badge")}</Badge> : null}
                      {server.allow_auto_approve ? (
                        <Badge tone="warning">{t("mcp.auto_badge")}</Badge>
                      ) : null}
                      <Badge tone="neutral">
                        {t("mcp.tool_count", { count: server.tool_count })}
                      </Badge>
                      {server.has_secret ? <Badge tone="neutral">{t("mcp.secret_set")}</Badge> : null}
                    </div>
                    <p className="muted small mono">{server.url}</p>
                    <p className="muted small">
                      {t("mcp.last_connection")} :{" "}
                      {server.last_connected_at
                        ? formatDate(server.last_connected_at, locale)
                        : t("common.none")}
                    </p>
                    {server.last_error ? (
                      <p className="danger-text small">{server.last_error}</p>
                    ) : null}
                  </div>
                  <div className="button-row compact">
                    {server.connected ? (
                      <button
                        type="button"
                        className="ghost small"
                        onClick={() => void disconnect(server)}
                        disabled={busy}
                      >
                        {t("mcp.disconnect")}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="primary small"
                        onClick={() => void connect(server)}
                        disabled={busy || !server.enabled}
                      >
                        {busy ? <Spinner /> : <Play size={13} aria-hidden="true" />}
                        {t("mcp.connect")}
                      </button>
                    )}
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void testConnection(server)}
                      disabled={busy || !server.enabled}
                    >
                      {t("mcp.test_connection")}
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void refreshTools(server)}
                      disabled={busy || !server.connected}
                    >
                      <Wrench size={13} aria-hidden="true" />
                      {t("mcp.refresh_tools")}
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void toggleServer(server)}
                    >
                      <Power size={13} aria-hidden="true" />
                      {server.enabled ? t("mcp.disable") : t("mcp.enable")}
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => startEdit(server)}
                    >
                      <Pencil size={13} aria-hidden="true" />
                      {t("common.edit")}
                    </button>
                    <button
                      type="button"
                      className="ghost small danger"
                      onClick={() => setToDelete(server)}
                    >
                      <Trash2 size={13} aria-hidden="true" />
                      {t("common.delete")}
                    </button>
                  </div>
                </header>

                {serverTools.length === 0 ? (
                  <p className="muted small">{t("mcp.no_tools")}</p>
                ) : (
                  <div className="table-wrap">
                    <table className="table">
                      <thead>
                        <tr>
                          <th scope="col">{t("mcp.tool")}</th>
                          <th scope="col">{t("mcp.category")}</th>
                          <th scope="col">{t("mcp.risk")}</th>
                          <th scope="col">{t("mcp.parameters")}</th>
                          <th scope="col">{t("mcp.enabled")}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {serverTools.map((tool) => (
                          <tr key={tool.name}>
                            <td>
                              <strong>{tool.name}</strong>
                              <div className="muted small">{tool.description}</div>
                            </td>
                            <td>
                              <Badge tone="neutral">
                                {t(`mcp.category_${tool.category}`, {
                                  defaultValue: tool.category,
                                })}
                              </Badge>
                            </td>
                            <td>
                              <Badge tone={riskTone(tool.risk)}>
                                {t(`mcp.risk_${tool.risk}`, { defaultValue: tool.risk })}
                              </Badge>
                            </td>
                            <td className="prewrap small">
                              {schemaFields(tool.input_schema)
                                .map((field) => `${field.name}: ${field.type}${field.required ? " *" : ""}`)
                                .join(", ") || "—"}
                            </td>
                            <td>
                              <button
                                type="button"
                                className="ghost small"
                                onClick={() => void toggleTool(server.id, tool.name, !tool.enabled)}
                                disabled={toolAction.busy}
                              >
                                {tool.enabled ? t("mcp.disable_tool") : t("mcp.enable_tool")}
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}

      <section className="card" aria-labelledby="mcp-call">
        <div className="section-heading">
          <h2 id="mcp-call">{t("mcp.call_title")}</h2>
        </div>
        <McpSecurityHint />
        <McpCallPanel caller="ui" />
      </section>

      <section className="card" aria-labelledby="mcp-history">
        <div className="section-heading">
          <h2 id="mcp-history">{t("mcp.history")}</h2>
          <RefreshCw
            size={15}
            aria-hidden="true"
            className="muted-icon"
            onClick={() => calls.reload()}
          />
        </div>
        {calls.loading ? (
          <LoadingRow label={t("common.loading")} />
        ) : (calls.data ?? []).length === 0 ? (
          <p className="muted small">{t("mcp.no_calls")}</p>
        ) : (
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">{t("mcp.server")}</th>
                  <th scope="col">{t("mcp.tool")}</th>
                  <th scope="col">{t("mcp.caller")}</th>
                  <th scope="col">{t("mcp.status")}</th>
                  <th scope="col">{t("mcp.duration")}</th>
                  <th scope="col">{t("mcp.date")}</th>
                  <th scope="col">{t("mcp.error")}</th>
                </tr>
              </thead>
              <tbody>
                {(calls.data ?? []).slice(0, 25).map((call: McpCallRecord) => (
                  <tr key={call.id}>
                    <td className="small">{call.server_id}</td>
                    <td className="small mono">{call.tool_name}</td>
                    <td className="small">{call.caller}</td>
                    <td>
                      <Badge
                        tone={
                          call.status === "completed"
                            ? "success"
                            : call.status === "denied" || call.status === "failed"
                              ? "danger"
                              : "warning"
                        }
                      >
                        {call.status}
                      </Badge>
                    </td>
                    <td className="small">
                      {call.duration_ms !== null ? `${call.duration_ms} ms` : "—"}
                    </td>
                    <td className="small">{formatDate(call.started_at, locale)}</td>
                    <td className="prewrap small">{call.error ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {exported ? (
          <details className="details">
            <summary>{t("mcp.export_preview")}</summary>
            <pre className="code-block">{exported}</pre>
          </details>
        ) : null}
      </section>

      <ConfirmDialog
        open={toDelete !== null}
        title={t("mcp.delete_confirm_title", { name: toDelete?.name ?? "" })}
        description={<p>{t("mcp.delete_confirm_text")}</p>}
        confirmLabel={t("common.delete")}
        danger
        busy={deleteAction.busy}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setToDelete(null)}
      />
    </>
  );
}
