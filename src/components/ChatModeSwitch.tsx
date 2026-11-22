import { Bot, MessageSquare } from "lucide-react";
import { useApp } from "../lib/store";

/** Mode du panneau de conversation : le chat 0.2.1 reste le mode par défaut. */
export type ChatMode = "chat" | "agent";

/**
 * Sélecteur « Chat | Agent ». Il ne change que le panneau de conversation :
 * la liste des conversations et la disposition du workspace restent en place.
 */
export function ChatModeSwitch({
  mode,
  onChange,
}: {
  mode: ChatMode;
  onChange: (mode: ChatMode) => void;
}) {
  const { t } = useApp();
  return (
    <div className="mode-switch" role="group" aria-label={t("agent_mode.mode_label")}>
      <button
        type="button"
        className={mode === "chat" ? "mode-option active" : "mode-option"}
        aria-pressed={mode === "chat"}
        onClick={() => onChange("chat")}
      >
        <MessageSquare size={13} aria-hidden="true" />
        {t("agent_mode.mode_chat")}
      </button>
      <button
        type="button"
        className={mode === "agent" ? "mode-option active" : "mode-option"}
        aria-pressed={mode === "agent"}
        onClick={() => onChange("agent")}
      >
        <Bot size={13} aria-hidden="true" />
        {t("agent_mode.mode_agent")}
      </button>
    </div>
  );
}
