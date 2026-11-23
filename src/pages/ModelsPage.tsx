import { useState } from "react";
import { BrainCircuit, Cloud, KeyRound, MonitorSmartphone, RefreshCw } from "lucide-react";
import {
  AppError,
  api,
  toAppError,
  type HealthReport,
  type ProviderView,
} from "../lib/api";
import { useAsync } from "../lib/hooks";
import { sortProviders } from "../lib/providers";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

type ProviderAction = "secret" | "clear" | "health" | "models" | "default" | "enabled";

interface Feedback {
  busy: ProviderAction | null;
  error: AppError | null;
  message: string | null;
  health: HealthReport | null;
}

const EMPTY_FEEDBACK: Feedback = { busy: null, error: null, message: null, health: null };

export function ModelsPage() {
  const { t, secretStatus, refreshConfig } = useApp();
  const providers = useAsync(() => api.providers.list(), []);
  const [feedback, setFeedback] = useState<Record<string, Feedback>>({});
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  const [modelChoices, setModelChoices] = useState<Record<string, string>>({});

  const feedbackFor = (providerId: string): Feedback => feedback[providerId] ?? EMPTY_FEEDBACK;

  const patchFeedback = (providerId: string, patch: Partial<Feedback>) => {
    setFeedback((previous) => ({
      ...previous,
      [providerId]: { ...(previous[providerId] ?? EMPTY_FEEDBACK), ...patch },
    }));
  };

  const perform = async (
    providerId: string,
    action: ProviderAction,
    task: () => Promise<void>,
  ) => {
    patchFeedback(providerId, { busy: action, error: null, message: null });
    try {
      await task();
    } catch (reason) {
      patchFeedback(providerId, { error: toAppError(reason) });
    } finally {
      patchFeedback(providerId, { busy: null });
    }
  };

  const secretAvailable = secretStatus?.state === "available";
  const providerList = sortProviders(providers.data ?? []);

  const saveSecret = async (provider: ProviderView) => {
    const secret = (keyInputs[provider.id] ?? "").trim();
    if (secret.length === 0) {
      patchFeedback(provider.id, {
        error: new AppError("invalid_input", t("models.key_empty"), true),
      });
      return;
    }
    await perform(provider.id, "secret", async () => {
      await api.providers.setSecret(provider.id, secret);
      setKeyInputs((previous) => ({ ...previous, [provider.id]: "" }));
      patchFeedback(provider.id, { message: t("models.secret_saved") });
      await providers.reload();
      await refreshConfig();
    });
  };

  const clearSecret = async (provider: ProviderView) => {
    await perform(provider.id, "clear", async () => {
      await api.providers.clearSecret(provider.id);
      patchFeedback(provider.id, { message: t("models.secret_cleared") });
      await providers.reload();
      await refreshConfig();
    });
  };

  const testConnection = async (provider: ProviderView) => {
    patchFeedback(provider.id, { health: null });
    await perform(provider.id, "health", async () => {
      const report = await api.providers.health(provider.id);
      patchFeedback(provider.id, { health: report });
    });
  };

  const discoverModels = async (provider: ProviderView) => {
    await perform(provider.id, "models", async () => {
      const models = await api.providers.models(provider.id);
      patchFeedback(provider.id, {
        message: t("models.model_count", { count: models.length }),
      });
      await providers.reload();
      await refreshConfig();
    });
  };

  const setDefaultModel = async (provider: ProviderView) => {
    const model = modelChoices[provider.id] ?? "";
    if (!model) {
      return;
    }
    await perform(provider.id, "default", async () => {
      await api.providers.setDefaultModel(provider.id, model);
      patchFeedback(provider.id, {
        message: t("models.default_saved", { model }),
      });
      await providers.reload();
      await refreshConfig();
    });
  };

  const toggleEnabled = async (provider: ProviderView) => {
    await perform(provider.id, "enabled", async () => {
      await api.providers.setEnabled(provider.id, !provider.enabled);
      patchFeedback(provider.id, { message: t("models.enabled_saved") });
      await providers.reload();
      await refreshConfig();
    });
  };

  return (
    <>
      <PageHeader
        title={t("models.title")}
        subtitle={t("models.models_hint")}
        actions={
          <button type="button" className="ghost" onClick={() => void providers.reload()}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        }
      />

      <div className="inline-status">
        <Badge tone={secretAvailable ? "success" : "danger"}>
          <KeyRound size={12} aria-hidden="true" />
          {t("models.secret_store")}
        </Badge>
        <span className="small muted">
          {secretStatus?.state === "available"
            ? t("models.secret_available", { backend: secretStatus.backend })
            : secretStatus?.state === "unavailable"
              ? t("models.secret_unavailable", { reason: secretStatus.reason })
              : t("common.unknown")}
        </span>
      </div>
      {secretStatus?.state === "unavailable" ? (
        <Notice tone="warning">{t("models.secret_unavailable_hint")}</Notice>
      ) : null}

      {providers.error ? <ErrorBanner error={providers.error} onRetry={providers.reload} /> : null}
      {providers.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : providers.data && providers.data.length === 0 ? (
        <EmptyState
          title={t("models.no_models")}
          description={<p>{t("models.models_hint")}</p>}
        />
      ) : (
        <div className="provider-grid">
          {providerList.map((provider) => {
            const state = feedbackFor(provider.id);
            const local = provider.kind === "ollama";
            const requiresSecret = !local;
            const keyStored = provider.secret_stored || provider.has_secret;
            const missingKey = requiresSecret && !keyStored;
            const models = provider.models;
            const choice = modelChoices[provider.id] ?? provider.default_model ?? "";
            const busy = state.busy;

            return (
              <article className="card provider-card" key={provider.id}>
                <header className="provider-header">
                  <div>
                    <div className="provider-title">
                      <h2>{provider.label}</h2>
                      <Badge tone={local ? "info" : "accent"}>
                        {local ? (
                          <MonitorSmartphone size={11} aria-hidden="true" />
                        ) : (
                          <Cloud size={11} aria-hidden="true" />
                        )}
                        {local ? t("models.local") : t("models.cloud")}
                      </Badge>
                      <Badge tone="neutral">{t(`models.kind_${provider.kind}`)}</Badge>
                      <Badge tone={provider.custom ? "info" : "accent"}>
                        {provider.custom
                          ? t("providers.badge_custom")
                          : t("providers.badge_integrated")}
                      </Badge>
                      <Badge tone={provider.enabled ? "success" : "danger"}>
                        {provider.enabled
                          ? t("providers.badge_active")
                          : t("providers.badge_inactive")}
                      </Badge>
                    </div>
                    <p className="mono small">{provider.base_url}</p>
                  </div>
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={provider.enabled}
                      onChange={() => void toggleEnabled(provider)}
                      disabled={busy !== null}
                      aria-label={provider.enabled ? t("models.disable") : t("models.enable")}
                    />
                    <span>{provider.enabled ? t("common.enabled") : t("common.disabled")}</span>
                  </label>
                </header>

                {!provider.enabled ? (
                  <p className="muted small">{t("models.provider_disabled_hint")}</p>
                ) : null}

                {state.error ? <ErrorBanner error={state.error} /> : null}
                {state.message ? <Notice tone="success">{state.message}</Notice> : null}

                {local ? (
                  <p className="muted small">{t("models.local_hint")}</p>
                ) : (
                  <div className="provider-secret">
                    <div className="inline-status">
                      <Badge tone={keyStored ? "success" : "warning"}>
                        {keyStored ? t("models.key_saved") : t("models.key_missing")}
                      </Badge>
                      {missingKey ? (
                        <Badge tone="warning">{t("models.key_required_badge")}</Badge>
                      ) : null}
                    </div>
                    <Field
                      label={t("models.new_key")}
                      htmlFor={`provider-key-${provider.id}`}
                      hint={`${t("models.api_key_hint")} ${t("models.never_shown")}`}
                    >
                      <div className="input-row">
                        <input
                          id={`provider-key-${provider.id}`}
                          type="password"
                          className="input"
                          autoComplete="off"
                          spellCheck={false}
                          value={keyInputs[provider.id] ?? ""}
                          placeholder={t("models.key_placeholder")}
                          disabled={!secretAvailable || busy !== null}
                          onChange={(event) =>
                            setKeyInputs((previous) => ({
                              ...previous,
                              [provider.id]: event.target.value,
                            }))
                          }
                        />
                        <button
                          type="button"
                          className="primary small"
                          onClick={() => void saveSecret(provider)}
                          disabled={
                            !secretAvailable ||
                            busy !== null ||
                            (keyInputs[provider.id] ?? "").trim().length === 0
                          }
                          aria-busy={busy === "secret"}
                        >
                          {busy === "secret" ? <Spinner /> : <KeyRound size={13} aria-hidden="true" />}
                          {t("models.save_key")}
                        </button>
                        <button
                          type="button"
                          className="ghost small"
                          onClick={() => void clearSecret(provider)}
                          disabled={!keyStored || busy !== null}
                        >
                          {t("models.clear_key")}
                        </button>
                      </div>
                    </Field>
                  </div>
                )}

                <div className="button-row">
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void testConnection(provider)}
                    disabled={busy !== null || !provider.enabled || missingKey}
                    aria-busy={busy === "health"}
                    title={missingKey ? t("models.requires_key") : t("models.test_hint")}
                  >
                    {busy === "health" ? <Spinner /> : <BrainCircuit size={13} aria-hidden="true" />}
                    {t("models.test_connection")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void discoverModels(provider)}
                    disabled={busy !== null || !provider.enabled || missingKey}
                    aria-busy={busy === "models"}
                    title={missingKey ? t("models.requires_key") : t("models.models_hint")}
                  >
                    {busy === "models" ? <Spinner /> : <RefreshCw size={13} aria-hidden="true" />}
                    {t("models.discover")}
                  </button>
                </div>

                {missingKey && provider.enabled ? (
                  <p className="muted small">{t("models.requires_key")}</p>
                ) : null}

                {state.health ? (
                  <Notice
                    tone={state.health.ok ? "success" : "error"}
                    title={
                      state.health.ok ? t("models.connection_ok") : t("models.connection_failed")
                    }
                  >
                    <p>
                      {t("models.latency", { ms: state.health.latency_ms })}
                      {" · "}
                      {state.health.detail}
                    </p>
                  </Notice>
                ) : null}

                <Field label={t("models.default_model")} htmlFor={`provider-model-${provider.id}`}>
                  <div className="input-row">
                    <select
                      id={`provider-model-${provider.id}`}
                      className="input"
                      value={choice}
                      disabled={models.length === 0 || busy !== null}
                      onChange={(event) =>
                        setModelChoices((previous) => ({
                          ...previous,
                          [provider.id]: event.target.value,
                        }))
                      }
                    >
                      <option value="">{t("models.select_placeholder")}</option>
                      {models.map((model) => (
                        <option key={model.id} value={model.id}>
                          {model.label || model.id}
                          {model.id === provider.default_model
                            ? ` — ${t("models.default_badge")}`
                            : ""}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      className="ghost small"
                      disabled={
                        busy !== null ||
                        choice.length === 0 ||
                        choice === provider.default_model
                      }
                      onClick={() => void setDefaultModel(provider)}
                      aria-busy={busy === "default"}
                    >
                      {busy === "default" ? <Spinner /> : null}
                      {t("models.set_default")}
                    </button>
                  </div>
                </Field>
                <p className="muted small">
                  {provider.default_model
                    ? `${t("models.default_badge")} : ${provider.default_model}`
                    : t("models.default_none")}
                </p>

                {models.length === 0 ? (
                  <p className="muted small">{t("models.no_models")}</p>
                ) : (
                  <div className="table-wrap model-table">
                    <table className="table">
                      <thead>
                        <tr>
                          <th scope="col">{t("common.id")}</th>
                          <th scope="col">{t("models.context_window")}</th>
                          <th scope="col">{t("models.supports_tools")}</th>
                          <th scope="col">{t("models.supports_vision")}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {models.map((model) => (
                          <tr key={model.id}>
                            <td className="mono">
                              {model.id}
                              {model.id === provider.default_model ? (
                                <Badge tone="accent">{t("models.default_badge")}</Badge>
                              ) : null}
                            </td>
                            <td>{model.context_window ?? "—"}</td>
                            <td>{model.supports_tools ? t("common.yes") : t("common.no")}</td>
                            <td>{model.supports_vision ? t("common.yes") : t("common.no")}</td>
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
    </>
  );
}
