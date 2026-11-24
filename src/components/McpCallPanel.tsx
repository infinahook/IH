import { useState } from "react";
import { Play, ShieldAlert, Wrench } from "lucide-react";
import { api, type McpRemoteCallOutcome, type McpRemoteServer, type McpRemoteTool } from "../lib/api";
import { riskTone, schemaFields, summarizeResult } from "../lib/mcp";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { Spinner } from "../components/EmptyState";

/** Panneau d'appel d'outil MCP réutilisable (page MCP et conversation). */
export function McpCallPanel({
  caller = "ui",
  compact = false,
  onResult,
}: {
  caller?: "ui" | "chat";
  compact?: boolean;
  onResult?: (result: McpRemoteCallOutcome) => void;
}) {
  const { t } = useApp();
  const servers = useAsync(() => api.mcpRemote.list(), []);
  const tools = useAsync(() => api.mcpRemote.tools(), []);
  const callAction = useAction();
  const [serverId, setServerId] = useState("");
  const [toolName, setToolName] = useState("");
  const [args, setArgs] = useState("{}");
  const [result, setResult] = useState<{ tool: string; outcome: NonNullable<McpRemoteCallOutcome["result"]> } | null>(null);
  const [pending, setPending] = useState<{
    approvalId: string;
    fingerprint: string | null;
    serverId: string;
    tool: string;
    args: unknown;
    outcome: McpRemoteCallOutcome;
  } | null>(null);
  const [localError, setLocalError] = useState<string | null>(null);

  const enabledServers = (servers.data ?? []).filter((server: McpRemoteServer) => server.enabled);
  const enabledTools = (tools.data ?? []).filter((tool: McpRemoteTool) => tool.enabled);
  const toolsOf = enabledTools.filter((tool) => tool.server_id === serverId);
  const selectedTool = enabledTools.find(
    (tool) => tool.server_id === serverId && tool.name === toolName,
  );

  const parseArguments = (): unknown | null => {
    try {
      const parsed: unknown = JSON.parse(args);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        setLocalError(t("mcp.object_required"));
        return null;
      }
      setLocalError(null);
      return parsed;
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      setLocalError(t("mcp.invalid_json", { message: detail }));
      return null;
    }
  };

  const invoke = async (approvalId?: string) => {
    const parsed = parseArguments();
    if (parsed === null) {
      return;
    }
    const outcome = await callAction.run(() =>
      api.mcpRemote.call({
        serverId,
        tool: toolName,
        arguments: parsed,
        caller,
        approvalId,
      }),
    );
    if (!outcome) {
      return;
    }
    if (outcome.status === "approval_required" && outcome.approval_id) {
      setPending({
        approvalId: outcome.approval_id,
        fingerprint: outcome.fingerprint,
        serverId,
        tool: toolName,
        args: parsed,
        outcome,
      });
      return;
    }
    if (outcome.result) {
      setResult({ tool: toolName, outcome: outcome.result });
      onResult?.(outcome);
    }
  };

  const approveAndRun = async () => {
    const request = pending;
    if (!request) {
      return;
    }
    const done = await callAction.run(async () => {
      await api.approvalResolve(request.approvalId, "approve");
      return true;
    });
    setPending(null);
    if (done) {
      await invoke(request.approvalId);
    }
  };

  return (
    <div className={compact ? "mcp-call compact" : "mcp-call"}>
      <div className="form-grid">
        <Field label={t("mcp.server")} htmlFor="mcp-call-server">
          <select
            id="mcp-call-server"
            className="input"
            value={serverId}
            onChange={(event) => {
              setServerId(event.target.value);
              setToolName("");
            }}
          >
            <option value="">—</option>
            {enabledServers.map((server) => (
              <option key={server.id} value={server.id}>
                {server.name}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("mcp.tool")} htmlFor="mcp-call-tool">
          <select
            id="mcp-call-tool"
            className="input"
            value={toolName}
            onChange={(event) => setToolName(event.target.value)}
          >
            <option value="">—</option>
            {toolsOf.map((tool) => (
              <option key={tool.name} value={tool.name}>
                {tool.name} ({t(`mcp.risk_${tool.risk}`, { defaultValue: tool.risk })})
              </option>
            ))}
          </select>
        </Field>
      </div>
      {selectedTool ? (
        <p className="muted small">
          {selectedTool.description}
          {schemaFields(selectedTool.input_schema).length > 0
            ? ` — ${schemaFields(selectedTool.input_schema)
                .map((field) => `${field.name}: ${field.type}${field.required ? " *" : ""}`)
                .join(", ")}`
            : ""}
        </p>
      ) : null}
      <Field label={t("mcp.arguments")} htmlFor="mcp-call-args" hint={t("mcp.arguments_hint")}>
        <textarea
          id="mcp-call-args"
          className="input textarea mono"
          rows={compact ? 3 : 4}
          spellCheck={false}
          value={args}
          onChange={(event) => setArgs(event.target.value)}
        />
      </Field>
      {localError ? <ErrorLine text={localError} /> : null}
      <div className="button-row">
        <button
          type="button"
          className="primary"
          onClick={() => void invoke()}
          disabled={callAction.busy || !serverId || !toolName}
          aria-busy={callAction.busy}
        >
          {callAction.busy ? <Spinner /> : <Play size={14} aria-hidden="true" />}
          {t("mcp.call")}
        </button>
        {enabledServers.length === 0 ? (
          <span className="muted small">{t("mcp.no_servers_hint")}</span>
        ) : serverId && toolsOf.length === 0 ? (
          <span className="muted small">{t("mcp.no_enabled_tools")}</span>
        ) : null}
      </div>
      {result ? (
        <Notice
          tone={result.outcome.is_error ? "error" : "success"}
          title={t("mcp.call_result", { tool: result.tool })}
        >
          <pre className="code-block">{summarizeResult(result.outcome.text)}</pre>
          {result.outcome.truncated ? (
            <p className="muted small">{t("mcp.result_truncated")}</p>
          ) : null}
        </Notice>
      ) : null}

      <ConfirmDialog
        open={pending !== null}
        title={t("mcp.approval_title")}
        description={
          <div>
            <p>
              <strong>
                {enabledServers.find((server) => server.id === pending?.serverId)?.name ??
                  pending?.serverId}
              </strong>{" "}
              · <strong>{pending?.tool}</strong>
            </p>
            <p className="small">
              {t("mcp.risk")} :{" "}
              <Badge tone={riskTone(selectedTool?.risk ?? "medium")}>
                {t(`mcp.risk_${selectedTool?.risk ?? "medium"}`, {
                  defaultValue: selectedTool?.risk ?? "medium",
                })}
              </Badge>
            </p>
            <p className="small">
              {t("mcp.effect")} :{" "}
              {t(`mcp.category_${selectedTool?.category ?? "write"}`, {
                defaultValue: selectedTool?.category ?? "write",
              })}
            </p>
            <p className="small mono">
              {t("mcp.fingerprint")} : {pending?.fingerprint}
            </p>
            <pre className="code-block">{JSON.stringify(pending?.args ?? {}, null, 2)}</pre>
            <p className="muted small">{t("mcp.approval_hint")}</p>
          </div>
        }
        confirmLabel={t("mcp.approval_confirm")}
        danger
        busy={callAction.busy}
        onConfirm={() => void approveAndRun()}
        onCancel={() => setPending(null)}
      />
    </div>
  );
}

function ErrorLine({ text }: { text: string }) {
  return (
    <p className="danger-text small" role="alert">
      <Wrench size={12} aria-hidden="true" /> {text}
    </p>
  );
}

/** Avertissement compact rappelant que l'autorisation n'est jamais implicite. */
export function McpSecurityHint() {
  const { t } = useApp();
  return (
    <p className="muted small">
      <ShieldAlert size={12} aria-hidden="true" /> {t("mcp.call_hint")}
    </p>
  );
}
