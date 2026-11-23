import { useEffect, useState } from "react";
import { Plus, RefreshCw, Trash2, Star } from "lucide-react";
import {
  api,
  toAppError,
  type AppError,
  type ModelEntry,
  type ProviderView,
} from "../lib/api";
import { providerRequiresSecret } from "../lib/providers";
import { useApp } from "../lib/store";
import { Badge } from "./Badge";
import { EmptyState, Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";
import { Field } from "./Field";
import { Modal } from "./Modal";
import { Notice } from "./Notice";

type Busy = "discover" | "add" | "remove" | "default" | null;

export function ProviderModelsModal({
  open,
  provider,
  onClose,
  onUpdated,
  onRefresh,
}: {
  open: boolean;
  provider: ProviderView | null;
  onClose: () => void;
  /** Remplace le fournisseur local par la vue renvoyée par le noyau. */
  onUpdated: (updated: ProviderView) => void;
  /** Recharge la liste réelle (découverte réseau, modèle par défaut). */
  onRefresh: () => Promise<void>;
}) {
  const { t } = useApp();
  const [busy, setBusy] = useState<Busy>(null);
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [messageTone, setMessageTone] = useState<"success" | "warning">("success");
  const [manualId, setManualId] = useState("");
  const [manualLabel, setManualLabel] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [pendingRemove, setPendingRemove] = useState<string | null>(null);
  const [failedAction, setFailedAction] = useState<Busy>(null);

  useEffect(() => {
    if (!open) {
      return;
    }
    setBusy(null);
    setActionError(null);
    setMessage(null);
    setMessageTone("success");
    setManualId("");
    setManualLabel("");
    setFormError(null);
    setPendingRemove(null);
    setFailedAction(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, provider?.id]);

  if (!provider) {
    return null;
  }

  const run = async (kind: Exclude<Busy, null>, task: () => Promise<void>) => {
    setBusy(kind);
    setActionError(null);
    setMessage(null);
    setFailedAction(null);
    try {
      await task();
    } catch (reason) {
      setActionError(toAppError(reason));
      setFailedAction(kind);
    } finally {
      setBusy(null);
    }
  };

  const discover = () =>
    run("discover", async () => {
      const found: ModelEntry[] = await api.providers.models(provider.id);
      // `models_list` persiste la découverte : on reflète tout de suite la liste.
      onUpdated({ ...provider, models: found });
      await onRefresh();
      setMessage(
        found.length === 0
          ? t("providers.models_discovery_empty")
          : t("providers.models_discovery_ok", { count: found.length }),
      );
      setMessageTone(found.length === 0 ? "warning" : "success");
    });

  const addModel = () => {
    const modelId = manualId.trim();
    if (modelId.length === 0) {
      setFormError(t("providers.models_id_required"));
      return;
    }
    setFormError(null);
    void run("add", async () => {
      const updated = await api.providers.addModel(
        provider.id,
        modelId,
        manualLabel.trim() || null,
      );
      onUpdated(updated);
      setManualId("");
      setManualLabel("");
      setMessage(t("providers.model_added", { model: modelId }));
    });
  };

  const removeModel = (modelId: string) =>
    run("remove", async () => {
      const updated = await api.providers.removeModel(provider.id, modelId);
      onUpdated(updated);
      setPendingRemove(null);
      setMessage(t("providers.model_removed", { model: modelId }));
    });

  const setDefault = (modelId: string) =>
    run("default", async () => {
      await api.providers.setDefaultModel(provider.id, modelId);
      await onRefresh();
      setMessage(t("providers.model_default_set", { model: modelId }));
    });

  const models = provider.models;
  const locked = busy !== null;

  return (
    <Modal
      open={open}
      title={t("providers.models_title", { label: provider.label })}
      onClose={onClose}
      size="wide"
      footer={
        <button type="button" className="ghost" onClick={onClose} disabled={locked}>
          {t("common.close")}
        </button>
      }
    >
      <div className="inline-status">
        <Badge tone={provider.custom ? "info" : "accent"}>
          {provider.custom ? t("providers.badge_custom") : t("providers.badge_integrated")}
        </Badge>
        <Badge tone={provider.enabled ? "success" : "danger"}>
          {provider.enabled ? t("providers.badge_active") : t("providers.badge_inactive")}
        </Badge>
        <Badge tone="neutral">{t(`models.kind_${provider.kind}`)}</Badge>
        <Badge tone={provider.default_model ? "accent" : "neutral"}>
          {provider.default_model
            ? t("providers.default_model_value", { model: provider.default_model })
            : t("providers.default_model_none")}
        </Badge>
      </div>
      <p className="mono small provider-url" title={provider.base_url}>
        {provider.base_url}
      </p>
      {provider.endpoint_path ? (
        <p className="muted small">
          {t("providers.endpoint_path_value", { path: provider.endpoint_path })}
        </p>
      ) : null}
      <p className="muted small">
        {t("providers.models_count", { count: models.length })}
        {" · "}
        {providerRequiresSecret(provider.kind)
          ? provider.has_secret
            ? t("providers.key_stored")
            : t("providers.key_none")
          : t("models.local_hint")}
      </p>

      {actionError ? <ErrorBanner error={actionError} onDismiss={() => setActionError(null)} /> : null}
      {actionError && failedAction === "discover" ? (
        <p className="muted small">{t("providers.models_discovery_error_hint")}</p>
      ) : null}
      {message ? <Notice tone={messageTone}>{message}</Notice> : null}
      {formError ? (
        <p className="field-error" role="alert">
          {formError}
        </p>
      ) : null}

      <div className="button-row">
        <button
          type="button"
          className="primary small"
          onClick={() => void discover()}
          disabled={locked}
          aria-busy={busy === "discover"}
          title={t("providers.discovery_hint")}
        >
          {busy === "discover" ? <Spinner /> : <RefreshCw size={13} aria-hidden="true" />}
          {busy === "discover" ? t("providers.models_discovering") : t("providers.models_discover")}
        </button>
        <span className="muted small">{t("providers.discovery_hint")}</span>
      </div>

      {models.length === 0 ? (
        <EmptyState
          title={t("providers.models_empty_title")}
          description={<p>{t("providers.models_empty_description")}</p>}
        />
      ) : (
        <div className="table-wrap model-table">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">{t("common.id")}</th>
                <th scope="col">{t("common.name")}</th>
                <th scope="col">{t("common.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {models.map((entry) => {
                const isDefault = entry.id === provider.default_model;
                const confirming = pendingRemove === entry.id;
                return (
                  <tr key={entry.id}>
                    <td className="mono">
                      {entry.id}
                      {isDefault ? <Badge tone="accent">{t("models.default_badge")}</Badge> : null}
                    </td>
                    <td>{entry.label || entry.id}</td>
                    <td>
                      <div className="button-row compact">
                        <button
                          type="button"
                          className="ghost small"
                          onClick={() => void setDefault(entry.id)}
                          disabled={locked || isDefault}
                          aria-busy={busy === "default"}
                          title={t("providers.set_default")}
                        >
                          <Star size={12} aria-hidden="true" />
                          {t("providers.set_default")}
                        </button>
                        {confirming ? (
                          <>
                            <span className="small muted">
                              {t("providers.model_remove_confirm", { model: entry.id })}
                            </span>
                            <button
                              type="button"
                              className="danger small"
                              onClick={() => void removeModel(entry.id)}
                              disabled={locked}
                              aria-busy={busy === "remove"}
                            >
                              {busy === "remove" ? <Spinner /> : <Trash2 size={12} aria-hidden="true" />}
                              {t("providers.model_confirm_yes")}
                            </button>
                            <button
                              type="button"
                              className="ghost small"
                              onClick={() => setPendingRemove(null)}
                              disabled={locked}
                            >
                              {t("common.cancel")}
                            </button>
                          </>
                        ) : (
                          <button
                            type="button"
                            className="ghost small"
                            onClick={() => setPendingRemove(entry.id)}
                            disabled={locked}
                          >
                            <Trash2 size={12} aria-hidden="true" />
                            {t("providers.model_remove")}
                          </button>
                        )}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <section className="provider-manual-model" aria-labelledby="providers-manual-model">
        <p className="field-label" id="providers-manual-model">
          {t("providers.models_add_manual")}
        </p>
        <div className="form-grid">
          <Field
            label={t("providers.model_id")}
            htmlFor="provider-model-id"
            error={formError}
            required
          >
            <input
              id="provider-model-id"
              className="input mono"
              value={manualId}
              disabled={locked}
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => {
                setManualId(event.target.value);
                setFormError(null);
              }}
            />
          </Field>
          <Field label={t("providers.model_label")} htmlFor="provider-model-label">
            <input
              id="provider-model-label"
              className="input"
              value={manualLabel}
              disabled={locked}
              onChange={(event) => setManualLabel(event.target.value)}
            />
          </Field>
        </div>
        <div className="button-row">
          <button
            type="button"
            className="ghost small"
            onClick={addModel}
            disabled={locked || manualId.trim().length === 0}
            aria-busy={busy === "add"}
          >
            {busy === "add" ? <Spinner /> : <Plus size={13} aria-hidden="true" />}
            {t("providers.model_add")}
          </button>
        </div>
      </section>

      <p className="muted small">{t("providers.manage_models_hint")}</p>
    </Modal>
  );
}
