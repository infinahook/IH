import { Settings2 } from "lucide-react";
import type { ModelEntry, ProviderView } from "../lib/api";
import { useApp } from "../lib/store";
import type { ProviderAvailability } from "../lib/ux";

/**
 * Barre compacte de la conversation : une seule ligne
 * `[ Provider ▼ ] [ Model ▼ ] [ ● état ] [ ⚙ ]`.
 *
 * Elle ne contient aucun libellé vertical ni bouton de découverte : la
 * configuration complète vit dans le drawer ouvert par ⚙.
 */
export function ChatProviderBar({
  providers,
  providerId,
  model,
  modelOptions,
  freeModelInput,
  loading,
  availability,
  onProviderChange,
  onModelChange,
  onOpenConfig,
}: {
  providers: readonly ProviderView[];
  providerId: string;
  model: string;
  modelOptions: readonly ModelEntry[];
  freeModelInput: boolean;
  loading: boolean;
  availability: ProviderAvailability;
  onProviderChange: (providerId: string) => void;
  /** `commit` demande la persistance du modèle par défaut. */
  onModelChange: (model: string, commit?: boolean) => void;
  onOpenConfig: () => void;
}) {
  const { t } = useApp();
  const enabled = providers.filter((entry) => entry.enabled);
  const disabled = providers.filter((entry) => !entry.enabled);
  const integrated = enabled.filter((entry) => !entry.custom);
  const custom = enabled.filter((entry) => entry.custom);
  const status = t(availability.labelKey);
  const hint = t(availability.hintKey);

  return (
    <div
      className="chat-provider-bar"
      role="group"
      aria-label={t("chat.provider_bar_label")}
    >
      <select
        id="chat-provider"
        className="input chat-provider-select"
        aria-label={t("chat.provider")}
        value={providerId}
        disabled={loading || enabled.length === 0}
        onChange={(event) => onProviderChange(event.target.value)}
      >
        {providerId ? null : <option value="">{t("chat.choose_provider")}</option>}
        {integrated.length > 0 ? (
          <optgroup label={t("providers.group_integrated")}>
            {integrated.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.label}
              </option>
            ))}
          </optgroup>
        ) : null}
        {custom.length > 0 ? (
          <optgroup label={t("providers.group_custom")}>
            {custom.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.label}
              </option>
            ))}
          </optgroup>
        ) : null}
        {disabled.length > 0 ? (
          <optgroup label={t("providers.group_disabled")}>
            {disabled.map((entry) => (
              <option key={entry.id} value={entry.id} disabled>
                {entry.label}
              </option>
            ))}
          </optgroup>
        ) : null}
      </select>

      {freeModelInput ? (
        <input
          id="chat-model-free"
          className="input chat-model-select"
          aria-label={t("chat.model")}
          value={model}
          placeholder={t("chat.model_free_placeholder")}
          disabled={loading}
          onChange={(event) => onModelChange(event.target.value)}
          onBlur={() => onModelChange(model, true)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onModelChange(model, true);
              event.currentTarget.blur();
            }
          }}
        />
      ) : (
        <select
          id="chat-model"
          className="input chat-model-select"
          aria-label={t("chat.model")}
          value={model}
          disabled={loading || modelOptions.length === 0}
          onChange={(event) => onModelChange(event.target.value, true)}
        >
          {model ? null : <option value="">{t("chat.choose_model")}</option>}
          {modelOptions.map((entry) => (
            <option key={entry.id} value={entry.id}>
              {entry.label || entry.id}
            </option>
          ))}
        </select>
      )}

      <span
        className={`provider-status provider-status-${availability.tone}`}
        title={hint}
        role="status"
        aria-label={status}
      >
        <span className="provider-status-dot" aria-hidden="true" />
        <span className="provider-status-text">{status}</span>
      </span>

      <button
        type="button"
        className="icon-button chat-config-button"
        onClick={onOpenConfig}
        title={t("chat.configure_provider")}
        aria-label={t("chat.configure_provider")}
      >
        <Settings2 size={15} aria-hidden="true" />
      </button>
    </div>
  );
}
