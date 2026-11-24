import { useState } from "react";
import { ShieldAlert, Wrench } from "lucide-react";
import type { ChatToolCallView } from "../lib/api";
import { maskSensitiveArguments } from "../lib/chat";
import { Badge } from "../components/Badge";
import { Spinner } from "../components/EmptyState";
import { useApp } from "../lib/store";

const STATUS_TONE: Record<string, "info" | "success" | "warning" | "danger" | "neutral"> = {
  pending: "warning",
  denied: "neutral",
  completed: "success",
  failed: "danger",
  invalid: "danger",
  expired: "neutral",
};

const RISK_TONE: Record<string, "info" | "success" | "warning" | "danger" | "neutral"> = {
  low: "success",
  medium: "warning",
  high: "danger",
  critical: "danger",
};

/** Cartes d'appels d'outils MCP demandés par le modèle, en attente de décision. */
export function ChatToolCards({
  cards,
  busyCallId,
  onDecide,
  onRefresh,
}: {
  cards: ChatToolCallView[];
  busyCallId: string | null;
  onDecide: (card: ChatToolCallView, decision: "approve" | "deny") => void;
  onRefresh: () => void;
}) {
  const { t } = useApp();
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});

  if (cards.length === 0) {
    return null;
  }
  return (
    <section className="card tool-call-cards" aria-label={t("chat.tool_call_title")}>
      <div className="section-heading">
        <h3>
          <Wrench size={16} aria-hidden="true" /> {t("chat.tool_call_title")}
        </h3>
        <div className="button-row compact">
          <Badge tone="info">{cards.length}</Badge>
          <button type="button" className="ghost small" onClick={onRefresh}>
            {t("chat.tool_call_refresh")}
          </button>
        </div>
      </div>
      <p className="muted small">{t("chat.tool_call_hint")}</p>
      <ul className="tool-call-list">
        {cards.map((card) => {
          const open = expanded[card.id] ?? false;
          const masked = maskSensitiveArguments(card.arguments);
          return (
            <li key={card.id} className={`tool-call-card tool-call-${card.status}`}>
              <div className="tool-call-head">
                <strong className="mono">
                  {card.server_id} · {card.tool}
                </strong>
                <Badge tone={RISK_TONE[card.risk] ?? "neutral"}>
                  {t("chat.mcp_proposal_risk")} : {card.risk}
                </Badge>
                <Badge tone={STATUS_TONE[card.status] ?? "neutral"}>
                  {t(`chat.tool_call_status_${card.status}`, {
                    defaultValue: card.status,
                  })}
                </Badge>
                <span className="muted small">#{card.provider_call_id}</span>
              </div>
              <button
                type="button"
                className="ghost small tool-call-toggle"
                aria-expanded={open}
                onClick={() =>
                  setExpanded((previous) => ({ ...previous, [card.id]: !open }))
                }
              >
                {open ? t("chat.tool_call_hide") : t("chat.tool_call_show")}
              </button>
              {open ? (
                <div className="tool-call-body">
                  <div className="field-label">{t("chat.tool_call_arguments")}</div>
                  <pre className="code-block">{JSON.stringify(masked, null, 2)}</pre>
                  {card.error ? (
                    <p className="small text-danger">{card.error}</p>
                  ) : null}
                  {card.result_text ? (
                    <>
                      <div className="field-label">{t("chat.tool_call_result")}</div>
                      <pre className="code-block tool-call-result">{card.result_text}</pre>
                    </>
                  ) : null}
                </div>
              ) : null}
              {card.status === "pending" ? (
                <div className="button-row compact">
                  <button
                    type="button"
                    className="primary small"
                    disabled={busyCallId !== null}
                    onClick={() => onDecide(card, "approve")}
                  >
                    {busyCallId === card.id ? <Spinner /> : <ShieldAlert size={14} />}
                    {t("chat.tool_call_approve")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    disabled={busyCallId !== null}
                    onClick={() => onDecide(card, "deny")}
                  >
                    {t("chat.tool_call_deny")}
                  </button>
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
