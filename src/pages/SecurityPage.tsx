import { useState } from "react";
import {
  AlertTriangle,
  Check,
  KeyRound,
  RefreshCw,
  ShieldCheck,
  TerminalSquare,
  X,
} from "lucide-react";
import {
  api,
  type ApprovalRequest,
  type ApprovalDecision,
  type CommandAssessment,
  type McpCallOutcome,
  type RiskLevel,
} from "../lib/api";
import { formatDate } from "../lib/format";
import { useTauriEvent } from "../lib/events";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge, riskTone } from "../components/Badge";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { JsonView } from "../components/JsonView";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

interface PermissionAskedEvent {
  approval_id: string;
  category: string;
  summary: string;
  detail: string;
}

export function SecurityPage() {
  const {
    t,
    locale,
    pendingToolCall,
    setPendingToolCall,
    navigate,
  } = useApp();
  const approvals = useAsync(() => api.approvalsPending(), []);
  const secrets = useAsync(() => api.secretStoreStatus(), []);
  const resolveAction = useAction();
  const toolAction = useAction();
  const assessAction = useAction();
  const [message, setMessage] = useState<string | null>(null);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [command, setCommand] = useState("");
  const [assessment, setAssessment] = useState<CommandAssessment | null>(null);
  const [toolOutcome, setToolOutcome] = useState<McpCallOutcome | null>(null);

  const permissionError = useTauriEvent<PermissionAskedEvent>(
    "ih://permission-asked",
    () => approvals.reload(),
  );

  const riskLabel = (risk: RiskLevel): string => t(`security.risk_${risk}`);

  const resolve = async (approval: ApprovalRequest, decision: ApprovalDecision) => {
    setPendingId(approval.id);
    setMessage(null);
    const done = await resolveAction.run(async () => {
      await api.approvalResolve(approval.id, decision);
      return true;
    });
    setPendingId(null);
    if (done) {
      let suffix = "";
      if (decision === "approve" && pendingToolCall?.approvalId === approval.id) {
        const pending = pendingToolCall;
        const outcome = await toolAction.run(() =>
          api.mcp.callTool(pending.serverId, pending.tool, pending.args, pending.approvalId),
        );
        if (outcome) {
          setToolOutcome(outcome);
          suffix = ` ${t("plugins.tool_call_result")}`;
        }
        setPendingToolCall(null);
      }
      if (decision === "deny" && pendingToolCall?.approvalId === approval.id) {
        setPendingToolCall(null);
      }
      setMessage(
        `${decision === "approve" ? t("security.approved_msg") : t("security.denied_msg")}${suffix}`,
      );
      approvals.reload();
    }
  };

  const approveAndRun = async () => {
    const pending = pendingToolCall;
    if (!pending) {
      return;
    }
    setMessage(null);
    const outcome = await toolAction.run(async () => {
      await api.approvalResolve(pending.approvalId, "approve");
      return api.mcp.callTool(pending.serverId, pending.tool, pending.args, pending.approvalId);
    });
    if (outcome) {
      setToolOutcome(outcome);
      setPendingToolCall(null);
      setMessage(t("plugins.tool_call_result"));
      approvals.reload();
    }
  };

  const classify = async () => {
    if (command.trim().length === 0) {
      return;
    }
    setAssessment(null);
    const result = await assessAction.run(() => api.classifyCommand(command));
    if (result) {
      setAssessment(result);
    }
  };

  return (
    <>
      <PageHeader
        title={t("security.title")}
        subtitle={t("overview.safety_text")}
        actions={
          <button type="button" className="ghost" onClick={() => approvals.reload()}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        }
      />

      {approvals.error ? <ErrorBanner error={approvals.error} onRetry={approvals.reload} /> : null}
      {resolveAction.error ? <ErrorBanner error={resolveAction.error} /> : null}
      {permissionError ? <ErrorBanner error={permissionError} /> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}

      {pendingToolCall ? (
        <section className="card" aria-labelledby="security-tool-call">
          <div className="section-heading">
            <h2 id="security-tool-call">{t("security.pending_tool_call")}</h2>
            <Badge tone="warning">
              <AlertTriangle size={12} aria-hidden="true" />
              {t("status.pending")}
            </Badge>
          </div>
          <p className="muted">{t("security.pending_tool_call_hint")}</p>
          <p className="mono small">
            {t("plugins.server")} : {pendingToolCall.serverId} · {t("plugins.tool_label")} :{" "}
            {pendingToolCall.tool}
          </p>
          <p className="muted small">{t("security.mcp_approval_hint")}</p>
          <div className="button-row">
            <button
              type="button"
              className="primary"
              onClick={() => void approveAndRun()}
              disabled={toolAction.busy}
              aria-busy={toolAction.busy}
            >
              {toolAction.busy ? <Spinner /> : <Check size={14} aria-hidden="true" />}
              {t("security.approve_and_run")}
            </button>
            <button type="button" className="ghost" onClick={() => navigate("plugins")}>
              {t("security.open_plugins")}
            </button>
          </div>
          <p className="muted small">{t("security.approve_tool_no_args")}</p>
          {toolAction.error ? <ErrorBanner error={toolAction.error} /> : null}
          {toolOutcome ? <JsonView value={toolOutcome.result ?? toolOutcome} maxHeight={260} /> : null}
        </section>
      ) : null}

      <section className="card" aria-labelledby="security-pending">
        <div className="section-heading">
          <h2 id="security-pending">{t("security.pending")}</h2>
          <Badge tone="neutral">
            {t("security.approvals_count", { count: approvals.data?.length ?? 0 })}
          </Badge>
        </div>
        {approvals.loading ? (
          <LoadingRow label={t("common.loading")} />
        ) : approvals.data && approvals.data.length > 0 ? (
          <ul className="approval-list">
            {approvals.data.map((approval) => (
              <li key={approval.id} className="approval-card">
                <div className="approval-head">
                  <div>
                    <h3>{approval.summary}</h3>
                    <p className="mono small">{approval.id}</p>
                  </div>
                  <div className="inline-status">
                    <Badge tone={riskTone(approval.risk)}>
                      {t("security.risk")} : {riskLabel(approval.risk)}
                    </Badge>
                    <Badge tone="neutral">{approval.category}</Badge>
                  </div>
                </div>
                <dl className="detail-grid">
                  <div>
                    <dt>{t("security.created")}</dt>
                    <dd>{formatDate(approval.created_at, locale)}</dd>
                  </div>
                  <div>
                    <dt>{t("security.expires")}</dt>
                    <dd>{formatDate(approval.expires_at, locale)}</dd>
                  </div>
                  <div>
                    <dt>{t("security.category")}</dt>
                    <dd className="mono">{approval.category}</dd>
                  </div>
                </dl>
                {approval.detail ? <pre className="code-block approval-detail">{approval.detail}</pre> : null}
                {approval.risk === "high" || approval.risk === "critical" ? (
                  <p className="small warning-text">{t("security.requires_confirmation")}</p>
                ) : null}
                <div className="button-row">
                  <button
                    type="button"
                    className="primary small"
                    onClick={() => void resolve(approval, "approve")}
                    disabled={resolveAction.busy}
                    aria-busy={pendingId === approval.id && resolveAction.busy}
                  >
                    {pendingId === approval.id && resolveAction.busy ? <Spinner /> : <Check size={13} aria-hidden="true" />}
                    {t("security.approve")}
                  </button>
                  <button
                    type="button"
                    className="danger small"
                    onClick={() => void resolve(approval, "deny")}
                    disabled={resolveAction.busy}
                  >
                    <X size={13} aria-hidden="true" />
                    {t("security.deny")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <EmptyState
            icon={<ShieldCheck size={22} aria-hidden="true" />}
            title={t("security.no_pending")}
            description={<p>{t("security.no_pending_hint")}</p>}
          />
        )}
      </section>

      <div className="grid grid-2">
        <section className="card" aria-labelledby="security-secrets">
          <div className="section-heading">
            <h2 id="security-secrets">{t("security.secrets_status")}</h2>
            <KeyRound size={17} aria-hidden="true" className="muted-icon" />
          </div>
          {secrets.error ? <ErrorBanner error={secrets.error} onRetry={secrets.reload} /> : null}
          {secrets.loading ? (
            <LoadingRow label={t("common.loading")} />
          ) : (
            <Notice
              tone={secrets.data?.state === "available" ? "success" : "error"}
              title={t("models.secret_store")}
            >
              <p className="mono">
                {secrets.data?.state === "available"
                  ? t("models.secret_available", { backend: secrets.data.backend })
                  : secrets.data?.state === "unavailable"
                    ? t("models.secret_unavailable", { reason: secrets.data.reason })
                    : t("common.unknown")}
              </p>
            </Notice>
          )}
          <p className="muted small">
            {secrets.data?.state === "unavailable"
              ? t("models.secret_unavailable_hint")
              : t("models.api_key_hint")}
          </p>
        </section>

        <section className="card" aria-labelledby="security-audit">
          <div className="section-heading">
            <h2 id="security-audit">{t("security.audit_log")}</h2>
            <ShieldCheck size={17} aria-hidden="true" className="muted-icon" />
          </div>
          <Notice tone="info" title={t("security.audit_unavailable")}>
            <p>{t("security.audit_hint")}</p>
          </Notice>
        </section>
      </div>

      <section className="card" aria-labelledby="security-tester">
        <div className="section-heading">
          <h2 id="security-tester">{t("security.command_tester")}</h2>
          <TerminalSquare size={17} aria-hidden="true" className="muted-icon" />
        </div>
        <p className="muted small">{t("security.tester_hint")}</p>
        <Field label={t("security.command_label")} htmlFor="security-command">
          <div className="input-row">
            <input
              id="security-command"
              className="input mono"
              value={command}
              placeholder={t("security.command_placeholder")}
              onChange={(event) => setCommand(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void classify();
                }
              }}
            />
            <button
              type="button"
              className="primary small"
              onClick={() => void classify()}
              disabled={assessAction.busy || command.trim().length === 0}
              aria-busy={assessAction.busy}
            >
              {assessAction.busy ? <Spinner /> : <ShieldCheck size={13} aria-hidden="true" />}
              {t("security.classify")}
            </button>
          </div>
        </Field>
        {assessAction.error ? <ErrorBanner error={assessAction.error} /> : null}
        {assessment ? (
          <div className="assessment" role="region" aria-live="polite">
            <div className="inline-status">
              <Badge tone={riskTone(assessment.risk)}>
                {t("security.risk_label", { risk: riskLabel(assessment.risk) })}
              </Badge>
              {assessment.requires_approval ? (
                <Badge tone="warning">{t("security.requires_approval")}</Badge>
              ) : null}
            </div>
            {assessment.risk === "critical" || assessment.risk === "high" ? (
              <p className="warning-text">{t("security.requires_confirmation")}</p>
            ) : null}
            <h3 className="section-title">{t("security.reasons")}</h3>
            {assessment.reasons.length === 0 ? (
              <p className="muted">{t("security.risk_none")}</p>
            ) : (
              <ul className="plain-list">
                {assessment.reasons.map((reason, index) => (
                  <li key={`${reason}-${index}`}>{reason}</li>
                ))}
              </ul>
            )}
          </div>
        ) : null}
      </section>
    </>
  );
}
