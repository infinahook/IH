import { useState } from "react";
import {
  CheckCircle2,
  Link2,
  Plug,
  Plus,
  RefreshCw,
  Shuffle,
  Trash2,
  Unlink,
} from "lucide-react";
import {
  AppError,
  api,
  outcomeApprovalId,
  toAppError,
  type InstalledExtension,
  type McpCallOutcome,
  type McpTool,
} from "../lib/api";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { JsonView } from "../components/JsonView";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

type ServerAction = "connect" | "disconnect" | "call" | "remove" | "enable";

interface ServerFeedback {
  busy: ServerAction | null;
  error: AppError | null;
  message: string | null;
}

const EMPTY_FEEDBACK: ServerFeedback = { busy: null, error: null, message: null };

function manifestArgs(server: InstalledExtension): string[] {
  const manifest = server.manifest;
  if (typeof manifest !== "object" || manifest === null) {
    return [];
  }
  const args = (manifest as { args?: unknown }).args;
  if (!Array.isArray(args)) {
    return [];
  }
  return args.filter((value): value is string => typeof value === "string");
}

export function PluginsPage() {
  const { t, pendingToolCall, setPendingToolCall, navigate } = useApp();
  const servers = useAsync(() => api.mcp.list(), []);
  const addServer = useAction();
  const [feedback, setFeedback] = useState<Record<string, ServerFeedback>>({});
  const [tools, setTools] = useState<Record<string, McpTool[]>>({});
  const [connectedServers, setConnectedServers] = useState<Record<string, boolean>>({});
  const [selectedTools, setSelectedTools] = useState<Record<string, string>>({});
  const [argumentsText, setArgumentsText] = useState<Record<string, string>>({});
  const [outcomes, setOutcomes] = useState<Record<string, McpCallOutcome | null>>({});
  const [toRemove, setToRemove] = useState<InstalledExtension | null>(null);
  const [form, setForm] = useState({ id: "", name: "", command: "", args: "" });

  const feedbackFor = (serverId: string): ServerFeedback =>
    feedback[serverId] ?? EMPTY_FEEDBACK;

  const patchFeedback = (serverId: string, patch: Partial<ServerFeedback>) => {
    setFeedback((previous) => ({
      ...previous,
      [serverId]: { ...(previous[serverId] ?? EMPTY_FEEDBACK), ...patch },
    }));
  };

  const runServer = async <T,>(
    serverId: string,
    action: ServerAction,
    task: () => Promise<T>,
  ): Promise<T | undefined> => {
    patchFeedback(serverId, { busy: action, error: null, message: null });
    try {
      return await task();
    } catch (reason) {
      patchFeedback(serverId, { error: toAppError(reason) });
      return undefined;
    } finally {
      patchFeedback(serverId, { busy: null });
    }
  };

  const handleAdd = async () => {
    const id = form.id.trim();
    const command = form.command.trim();
    if (!id || !command) {
      addServer.setError(
        new AppError("invalid_input", t("plugins.id_command_required"), true),
      );
      return;
    }
    const args = form.args.trim().length > 0 ? form.args.trim().split(/\s+/) : [];
    const done = await addServer.run(async () => {
      await api.mcp.add(id, form.name.trim() || id, command, args);
      return true;
    });
    if (done) {
      setForm({ id: "", name: "", command: "", args: "" });
      servers.reload();
    }
  };

  const toggleEnabled = async (server: InstalledExtension) => {
    const done = await runServer(server.id, "enable", async () => {
      await api.mcp.setEnabled(server.id, !server.enabled);
      return true;
    });
    if (done) {
      servers.reload();
      if (server.enabled) {
        setConnectedServers((previous) => ({ ...previous, [server.id]: false }));
        setTools((previous) => ({ ...previous, [server.id]: [] }));
      }
    }
  };

  const connect = async (server: InstalledExtension) => {
    const loaded = await runServer(server.id, "connect", () => api.mcp.connect(server.id, "IH"));
    if (loaded) {
      setTools((previous) => ({ ...previous, [server.id]: loaded }));
      setConnectedServers((previous) => ({ ...previous, [server.id]: true }));
      setSelectedTools((previous) => ({
        ...previous,
        [server.id]: loaded[0]?.name ?? "",
      }));
      setArgumentsText((previous) => ({ ...previous, [server.id]: "{}" }));
      setOutcomes((previous) => ({ ...previous, [server.id]: null }));
      patchFeedback(server.id, {
        message: t("plugins.connected_tools", { count: loaded.length }),
      });
    }
  };

  const disconnect = async (server: InstalledExtension) => {
    const done = await runServer(server.id, "disconnect", async () => {
      await api.mcp.disconnect(server.id);
      return true;
    });
    if (done) {
      setTools((previous) => ({ ...previous, [server.id]: [] }));
      setConnectedServers((previous) => ({ ...previous, [server.id]: false }));
      setOutcomes((previous) => ({ ...previous, [server.id]: null }));
      patchFeedback(server.id, { message: t("plugins.disconnected") });
    }
  };

  const parseArguments = (serverId: string): Record<string, unknown> | null => {
    const raw = (argumentsText[serverId] ?? "").trim();
    if (raw.length === 0) {
      return {};
    }
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new Error(t("plugins.object_required"));
      }
      return parsed as Record<string, unknown>;
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      patchFeedback(serverId, {
        error: new AppError("invalid_input", t("plugins.invalid_json", { message }), true),
      });
      return null;
    }
  };

  const callTool = async (server: InstalledExtension) => {
    const tool = selectedTools[server.id] ?? "";
    if (!tool) {
      return;
    }
    const args = parseArguments(server.id);
    if (args === null) {
      return;
    }
    const outcome = await runServer(server.id, "call", () =>
      api.mcp.callTool(server.id, tool, args),
    );
    if (!outcome) {
      return;
    }
    const approvalId = outcomeApprovalId(outcome);
    if (outcome.status === "approval_required" && approvalId) {
      setPendingToolCall({
        approvalId,
        serverId: server.id,
        tool,
        args,
      });
      setOutcomes((previous) => ({ ...previous, [server.id]: outcome }));
      patchFeedback(server.id, { message: t("plugins.requires_approval") });
      return;
    }
    setOutcomes((previous) => ({ ...previous, [server.id]: outcome }));
    patchFeedback(server.id, { message: t("plugins.tool_call_result") });
  };

  const approveAndRetry = async () => {
    const pending = pendingToolCall;
    if (!pending) {
      return;
    }
    const outcome = await runServer(pending.serverId, "call", async () => {
      await api.approvalResolve(pending.approvalId, "approve");
      return api.mcp.callTool(pending.serverId, pending.tool, pending.args, pending.approvalId);
    });
    if (outcome) {
      setOutcomes((previous) => ({ ...previous, [pending.serverId]: outcome }));
      setPendingToolCall(null);
      patchFeedback(pending.serverId, { message: t("plugins.tool_call_result") });
    }
  };

  const retryApproved = async () => {
    const pending = pendingToolCall;
    if (!pending) {
      return;
    }
    const outcome = await runServer(pending.serverId, "call", () =>
      api.mcp.callTool(pending.serverId, pending.tool, pending.args, pending.approvalId),
    );
    if (outcome) {
      setOutcomes((previous) => ({ ...previous, [pending.serverId]: outcome }));
      setPendingToolCall(null);
      patchFeedback(pending.serverId, { message: t("plugins.tool_call_result") });
    }
  };

  const removeServer = async () => {
    if (!toRemove) {
      return;
    }
    const target = toRemove;
    const done = await runServer(target.id, "remove", async () => {
      if (connectedServers[target.id]) {
        await api.mcp.disconnect(target.id);
      }
      await api.mcp.remove(target.id);
      return true;
    });
    if (done) {
      setToRemove(null);
      setConnectedServers((previous) => ({ ...previous, [target.id]: false }));
      setTools((previous) => ({ ...previous, [target.id]: [] }));
      setOutcomes((previous) => ({ ...previous, [target.id]: null }));
      servers.reload();
    }
  };

  return (
    <>
      <PageHeader
        title={t("plugins.title")}
        subtitle={t("plugins.call_hint")}
        actions={
          <button type="button" className="ghost" onClick={() => servers.reload()}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        }
      />

      {pendingToolCall ? (
        <div className="banner banner-warning" role="status">
          <Plug size={18} aria-hidden="true" />
          <div className="banner-content">
            <strong>{t("plugins.approval_pending_title")}</strong>
            <p className="banner-detail">
              {t("plugins.approval_pending_text")} · {t("plugins.server")} : {pendingToolCall.serverId}{" "}
              · {t("plugins.tool_label")} : {pendingToolCall.tool}
            </p>
            <p className="banner-meta">{t("plugins.approved_retry_hint")}</p>
          </div>
          <div className="banner-actions">
            <button type="button" className="primary small" onClick={() => void approveAndRetry()}>
              <CheckCircle2 size={14} aria-hidden="true" />
              {t("plugins.approve_and_retry")}
            </button>
            <button type="button" className="ghost small" onClick={() => void retryApproved()}>
              <Shuffle size={14} aria-hidden="true" />
              {t("plugins.retry")}
            </button>
            <button type="button" className="ghost small" onClick={() => navigate("security")}>
              {t("plugins.open_security")}
            </button>
          </div>
        </div>
      ) : null}

      {servers.error ? <ErrorBanner error={servers.error} onRetry={servers.reload} /> : null}
      {addServer.error ? <ErrorBanner error={addServer.error} /> : null}

      <section className="card" aria-labelledby="plugins-add">
        <div className="section-heading">
          <h2 id="plugins-add">{t("plugins.add_server")}</h2>
        </div>
        <p className="muted small">{t("plugins.add_hint")}</p>
        <div className="form-grid">
          <Field label={t("plugins.server_id")} htmlFor="mcp-id" required>
            <input
              id="mcp-id"
              className="input"
              value={form.id}
              onChange={(event) => setForm({ ...form, id: event.target.value })}
              autoComplete="off"
            />
          </Field>
          <Field label={t("plugins.server_name")} htmlFor="mcp-name">
            <input
              id="mcp-name"
              className="input"
              value={form.name}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
              autoComplete="off"
            />
          </Field>
          <Field label={t("plugins.command")} htmlFor="mcp-command" required>
            <input
              id="mcp-command"
              className="input mono"
              value={form.command}
              onChange={(event) => setForm({ ...form, command: event.target.value })}
              autoComplete="off"
            />
          </Field>
          <Field label={t("plugins.args")} htmlFor="mcp-args" hint={t("plugins.args_hint")}>
            <input
              id="mcp-args"
              className="input mono"
              value={form.args}
              onChange={(event) => setForm({ ...form, args: event.target.value })}
              autoComplete="off"
            />
          </Field>
        </div>
        <div className="button-row">
          <button
            type="button"
            className="primary"
            onClick={() => void handleAdd()}
            disabled={addServer.busy}
            aria-busy={addServer.busy}
          >
            {addServer.busy ? <Spinner /> : <Plus size={14} aria-hidden="true" />}
            {t("plugins.add_server")}
          </button>
        </div>
      </section>

      {servers.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : servers.data && servers.data.length === 0 ? (
        <EmptyState
          icon={<Plug size={22} aria-hidden="true" />}
          title={t("plugins.no_servers")}
          description={<p>{t("plugins.no_servers_hint")}</p>}
        />
      ) : (
        <div className="server-list">
          {(servers.data ?? []).map((server) => {
            const state = feedbackFor(server.id);
            const connected = connectedServers[server.id] === true;
            const serverTools = tools[server.id] ?? [];
            const selectedTool = selectedTools[server.id] ?? "";
            const outcome = outcomes[server.id] ?? null;
            const busy = state.busy;
            const args = manifestArgs(server);
            const tool = serverTools.find((entry) => entry.name === selectedTool);

            return (
              <article className="card server-card" key={server.id}>
                <header className="provider-header">
                  <div>
                    <div className="provider-title">
                      <h2>{server.name}</h2>
                      <Badge tone={connected ? "success" : "neutral"}>
                        {connected ? t("status.connected") : t("status.disconnected")}
                      </Badge>
                      <Badge tone="neutral">stdio</Badge>
                    </div>
                    <p className="mono small">
                      {server.id} · {server.source}
                      {args.length > 0 ? ` ${args.join(" ")}` : ""}
                    </p>
                  </div>
                  <div className="button-row compact">
                    {connected ? (
                      <button
                        type="button"
                        className="ghost small"
                        onClick={() => void disconnect(server)}
                        disabled={busy !== null}
                      >
                        {busy === "disconnect" ? <Spinner /> : <Unlink size={13} aria-hidden="true" />}
                        {t("plugins.disconnect")}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="primary small"
                        onClick={() => void connect(server)}
                        disabled={busy !== null || !server.enabled}
                        title={server.enabled ? undefined : t("plugins.enabled_hint")}
                        aria-busy={busy === "connect"}
                      >
                        {busy === "connect" ? <Spinner /> : <Link2 size={13} aria-hidden="true" />}
                        {t("plugins.connect")}
                      </button>
                    )}
                    <label className="checkbox-row drawer-checkbox">
                      <input
                        type="checkbox"
                        checked={server.enabled}
                        disabled={busy !== null}
                        onChange={() => void toggleEnabled(server)}
                      />
                      <span>{t("plugins.enabled")}</span>
                    </label>
                    <button
                      type="button"
                      className="ghost small danger-text"
                      onClick={() => setToRemove(server)}
                      disabled={busy !== null}
                    >
                      <Trash2 size={13} aria-hidden="true" />
                      {t("common.delete")}
                    </button>
                  </div>
                </header>

                {state.error ? <ErrorBanner error={state.error} /> : null}
                {state.message ? <Notice tone="success">{state.message}</Notice> : null}

                {!connected ? (
                  <p className="muted small">{t("plugins.no_connection")}</p>
                ) : serverTools.length === 0 ? (
                  <p className="muted small">{t("plugins.no_tools")}</p>
                ) : (
                  <>
                    <Field label={t("plugins.tool_label")} htmlFor={`tool-${server.id}`}>
                      <select
                        id={`tool-${server.id}`}
                        className="input"
                        value={selectedTool}
                        onChange={(event) =>
                          setSelectedTools((previous) => ({
                            ...previous,
                            [server.id]: event.target.value,
                          }))
                        }
                      >
                        {serverTools.map((entry) => (
                          <option key={entry.name} value={entry.name}>
                            {entry.name}
                          </option>
                        ))}
                      </select>
                    </Field>
                    {tool?.description ? <p className="muted small">{tool.description}</p> : null}
                    {tool ? (
                      <details className="details">
                        <summary>{t("plugins.schema")}</summary>
                        <JsonView value={tool.inputSchema} maxHeight={220} />
                      </details>
                    ) : null}
                    <Field
                      label={t("plugins.arguments_json")}
                      htmlFor={`args-${server.id}`}
                      hint={t("plugins.arguments_hint")}
                    >
                      <textarea
                        id={`args-${server.id}`}
                        className="input textarea mono"
                        rows={4}
                        spellCheck={false}
                        value={argumentsText[server.id] ?? "{}"}
                        onChange={(event) =>
                          setArgumentsText((previous) => ({
                            ...previous,
                            [server.id]: event.target.value,
                          }))
                        }
                      />
                    </Field>
                    <div className="button-row">
                      <button
                        type="button"
                        className="primary small"
                        onClick={() => void callTool(server)}
                        disabled={busy !== null || !selectedTool}
                        aria-busy={busy === "call"}
                      >
                        {busy === "call" ? <Spinner /> : <Shuffle size={13} aria-hidden="true" />}
                        {t("plugins.call_tool")}
                      </button>
                      <span className="muted small">{t("plugins.call_hint")}</span>
                    </div>
                    {outcome ? (
                      <div className="result-block">
                        <h3 className="section-title">{t("plugins.result")}</h3>
                        {outcome.status === "approval_required" ? (
                          <Notice tone="warning" title={t("plugins.approval_pending_title")}>
                            <p>{t("plugins.approval_pending_text")}</p>
                          </Notice>
                        ) : (
                          <JsonView value={outcome.result ?? outcome} maxHeight={320} />
                        )}
                      </div>
                    ) : null}
                  </>
                )}
              </article>
            );
          })}
        </div>
      )}

      <ConfirmDialog
        open={toRemove !== null}
        title={t("plugins.remove_confirm_title", { name: toRemove?.name ?? "" })}
        description={<p>{t("plugins.remove_confirm_text")}</p>}
        confirmLabel={t("common.delete")}
        onCancel={() => setToRemove(null)}
        onConfirm={() => void removeServer()}
        busy={feedbackFor(toRemove?.id ?? "").busy === "remove"}
        danger
      />
    </>
  );
}
