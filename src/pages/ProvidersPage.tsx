import { useEffect, useMemo, useState } from "react";
import {
  Boxes,
  KeyRound,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Server,
  Star,
  Trash2,
  Wifi,
} from "lucide-react";
import {
  api,
  toAppError,
  type AppError,
  type ConnectionTest,
  type ProviderView,
} from "../lib/api";
import {
  connectionNoticeTone,
  connectionStatusKey,
  connectionStatusTone,
  headerList,
  providerRequiresSecret,
  sortProviders,
} from "../lib/providers";
import { useAsync } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProviderFormModal } from "../components/ProviderFormModal";
import { ProviderModelsModal } from "../components/ProviderModelsModal";

interface TestState {
  busy: boolean;
  result: ConnectionTest | null;
  error: AppError | null;
}

interface Feedback {
  error: AppError | null;
  message: string | null;
}

const EMPTY_TEST: TestState = { busy: false, result: null, error: null };
const EMPTY_FEEDBACK: Feedback = { error: null, message: null };

type ModelTarget = { provider: ProviderView; modelId: string };

/**
 * Fournisseurs en maître/détail : la colonne gauche ne montre que l'essentiel
 * (nom, protocole, état, nombre de modèles), la colonne droite le détail du
 * fournisseur sélectionné. Aucun tableau ne liste tous les modèles à la fois.
 */
export function ProvidersPage() {
  const { t, config, refreshConfig } = useApp();
  const providers = useAsync(() => api.providers.list(), []);
  const [tests, setTests] = useState<Record<string, TestState>>({});
  const [feedback, setFeedback] = useState<Record<string, Feedback>>({});
  const [toggleBusy, setToggleBusy] = useState<string | null>(null);
  const [modelBusy, setModelBusy] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [formProvider, setFormProvider] = useState<ProviderView | null>(null);
  const [modelsFor, setModelsFor] = useState<string | null>(null);
  const [removeTarget, setRemoveTarget] = useState<ProviderView | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [removeError, setRemoveError] = useState<AppError | null>(null);
  const [modelToRemove, setModelToRemove] = useState<ModelTarget | null>(null);
  const [pageMessage, setPageMessage] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const list = useMemo(() => sortProviders(providers.data ?? []), [providers.data]);
  const selected = list.find((entry) => entry.id === selectedId) ?? list[0] ?? null;
  const modelsProvider = providers.data?.find((entry) => entry.id === modelsFor) ?? null;

  useEffect(() => {
    if (list.length === 0) {
      if (selectedId !== null) {
        setSelectedId(null);
      }
      return;
    }
    if (!selectedId || !list.some((entry) => entry.id === selectedId)) {
      setSelectedId(list[0].id);
    }
  }, [list, selectedId]);

  const testFor = (providerId: string): TestState => tests[providerId] ?? EMPTY_TEST;
  const feedbackFor = (providerId: string): Feedback => feedback[providerId] ?? EMPTY_FEEDBACK;

  const patchFeedback = (providerId: string, patch: Partial<Feedback>) => {
    setFeedback((current) => ({
      ...current,
      [providerId]: { ...(current[providerId] ?? EMPTY_FEEDBACK), ...patch },
    }));
  };

  const reload = async () => {
    providers.reload();
    await refreshConfig();
  };

  const openCreate = () => {
    setPageMessage(null);
    setFormProvider(null);
    setFormOpen(true);
  };

  const openEdit = (provider: ProviderView) => {
    setPageMessage(null);
    setFormProvider(provider);
    setFormOpen(true);
  };

  const handleSaved = async (saved: ProviderView) => {
    setPageMessage(null);
    setFormOpen(false);
    setFormProvider(null);
    setSelectedId(saved.id);
    patchFeedback(saved.id, {
      message: formProvider
        ? t("providers.updated", { label: saved.label })
        : t("providers.created", { label: saved.label }),
      error: null,
    });
    await reload();
  };

  const testConnection = async (provider: ProviderView) => {
    setTests((current) => ({
      ...current,
      [provider.id]: { busy: true, result: null, error: null },
    }));
    try {
      const result = await api.providers.test(provider.id);
      setTests((current) => ({
        ...current,
        [provider.id]: { busy: false, result, error: null },
      }));
    } catch (reason) {
      setTests((current) => ({
        ...current,
        [provider.id]: { busy: false, result: null, error: toAppError(reason) },
      }));
    }
  };

  const toggleEnabled = async (provider: ProviderView) => {
    setToggleBusy(provider.id);
    patchFeedback(provider.id, { error: null, message: null });
    try {
      await api.providers.setEnabled(provider.id, !provider.enabled);
      patchFeedback(provider.id, {
        message: provider.enabled ? t("providers.disabled_saved") : t("providers.enabled_saved"),
      });
      await reload();
    } catch (reason) {
      patchFeedback(provider.id, { error: toAppError(reason) });
    } finally {
      setToggleBusy(null);
    }
  };

  const confirmDelete = async () => {
    if (!removeTarget) {
      return;
    }
    const removedLabel = removeTarget.label;
    setRemoveBusy(true);
    setRemoveError(null);
    try {
      await api.providers.remove(removeTarget.id);
      setRemoveTarget(null);
      setPageMessage(t("providers.deleted", { label: removedLabel }));
      setTests((current) => {
        const next = { ...current };
        delete next[removeTarget.id];
        return next;
      });
      setSelectedId(null);
      await reload();
    } catch (reason) {
      setRemoveError(toAppError(reason));
    } finally {
      setRemoveBusy(false);
    }
  };

  const setDefaultModel = async (provider: ProviderView, modelId: string) => {
    setModelBusy(`${provider.id}:${modelId}`);
    patchFeedback(provider.id, { error: null, message: null });
    try {
      await api.providers.setDefaultModel(provider.id, modelId);
      patchFeedback(provider.id, {
        message: t("providers.model_default_set", { model: modelId }),
      });
      await reload();
    } catch (reason) {
      patchFeedback(provider.id, { error: toAppError(reason) });
    } finally {
      setModelBusy(null);
    }
  };

  const confirmRemoveModel = async () => {
    if (!modelToRemove) {
      return;
    }
    const target = modelToRemove;
    setModelBusy(`${target.provider.id}:${target.modelId}`);
    setModelToRemove(null);
    try {
      await api.providers.removeModel(target.provider.id, target.modelId);
      patchFeedback(target.provider.id, {
        message: t("providers.model_removed", { model: target.modelId }),
      });
      await reload();
    } catch (reason) {
      patchFeedback(target.provider.id, { error: toAppError(reason) });
    } finally {
      setModelBusy(null);
    }
  };

  const handleModelUpdate = (updated: ProviderView) => {
    providers.setData((current) =>
      current ? current.map((entry) => (entry.id === updated.id ? updated : entry)) : current,
    );
    // La liste des modèles vit aussi dans `config_get` (page Paramètres, formulaire).
    void refreshConfig();
  };

  const refreshModels = async () => {
    await reload();
  };

  const state = selected ? testFor(selected.id) : EMPTY_TEST;
  const info = selected ? feedbackFor(selected.id) : EMPTY_FEEDBACK;
  const requiresSecret = selected ? providerRequiresSecret(selected.kind) : false;
  const keyStored =
    selected !== null && (selected.secret_stored || selected.has_secret);
  const missingKey = requiresSecret && !keyStored;
  const cannotTest = selected === null || !selected.enabled || missingKey || state.busy;
  const storedHeaders = selected
    ? headerList(config?.providers.find((entry) => entry.id === selected.id)?.headers)
    : [];

  return (
    <>
      <PageHeader
        title={t("providers.title")}
        subtitle={t("providers.subtitle")}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => void providers.reload()}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            <button type="button" className="primary" onClick={openCreate}>
              <Plus size={15} aria-hidden="true" />
              {t("providers.add")}
            </button>
          </>
        }
      />

      {pageMessage ? <Notice tone="success">{pageMessage}</Notice> : null}
      {providers.error ? (
        <ErrorBanner error={providers.error} onRetry={providers.reload} />
      ) : null}
      {providers.loading && providers.data === null ? (
        <LoadingRow label={t("common.loading")} />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<Server size={22} aria-hidden="true" />}
          title={t("providers.empty_title")}
          description={<p>{t("providers.empty_description")}</p>}
          action={
            <button type="button" className="primary" onClick={openCreate}>
              <Plus size={15} aria-hidden="true" />
              {t("providers.add")}
            </button>
          }
        />
      ) : (
        <div className="provider-master-detail">
          <aside className="provider-master card" aria-label={t("providers.title")}>
            <div className="section-heading provider-master-head">
              <h2>{t("providers.title")}</h2>
              <span className="small muted">{t("providers.list_count", { count: list.length })}</span>
            </div>
            <ul className="provider-list">
              {list.map((provider) => {
                const active = provider.id === selected?.id;
                const keyOk =
                  !providerRequiresSecret(provider.kind) ||
                  provider.secret_stored ||
                  provider.has_secret;
                const dot = !provider.enabled
                  ? "status-dot-off"
                  : keyOk
                    ? "status-dot-on"
                    : "status-dot-warn";
                return (
                  <li key={provider.id}>
                    <button
                      type="button"
                      className={active ? "provider-list-row active" : "provider-list-row"}
                      aria-current={active ? "true" : undefined}
                      onClick={() => setSelectedId(provider.id)}
                    >
                      <span className={`status-dot ${dot}`} aria-hidden="true" />
                      <span className="provider-list-main">
                        <span className="provider-list-label">{provider.label}</span>
                        <span className="provider-list-meta">
                          {t(`models.kind_${provider.kind}`)} ·{" "}
                          {t("providers.models_count", { count: provider.models.length })}
                        </span>
                      </span>
                      <span
                        className={
                          provider.enabled
                            ? "provider-list-state"
                            : "provider-list-state off"
                        }
                      >
                        {provider.enabled
                          ? t("providers.badge_active")
                          : t("providers.badge_inactive")}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
            <p className="muted small provider-master-hint">{t("providers.select_hint")}</p>
          </aside>

          {selected ? (
            <section className="provider-detail card" aria-labelledby="provider-detail-title">
              <header className="provider-detail-head">
                <div className="provider-detail-identity">
                  <div className="provider-title">
                    <h2 id="provider-detail-title">{selected.label}</h2>
                    <Badge tone={selected.custom ? "info" : "accent"}>
                      {selected.custom
                        ? t("providers.badge_custom")
                        : t("providers.badge_integrated")}
                    </Badge>
                    <Badge tone={selected.enabled ? "success" : "danger"}>
                      {selected.enabled
                        ? t("providers.badge_active")
                        : t("providers.badge_inactive")}
                    </Badge>
                    <Badge tone={keyStored ? "success" : "neutral"}>
                      <KeyRound size={11} aria-hidden="true" />
                      {keyStored ? t("providers.key_stored") : t("providers.key_none")}
                    </Badge>
                  </div>
                  <p className="mono small provider-url" title={selected.base_url}>
                    {selected.base_url}
                  </p>
                </div>
                <div className="provider-detail-actions">
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => openEdit(selected)}
                  >
                    <Pencil size={13} aria-hidden="true" />
                    {t("providers.edit")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void testConnection(selected)}
                    disabled={cannotTest}
                    aria-busy={state.busy}
                    title={
                      !selected.enabled
                        ? t("providers.test_disabled")
                        : missingKey
                          ? t("providers.test_no_secret")
                          : t("providers.test_hint")
                    }
                  >
                    {state.busy ? <Spinner /> : <Wifi size={13} aria-hidden="true" />}
                    {t("providers.test")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void toggleEnabled(selected)}
                    disabled={toggleBusy === selected.id}
                    aria-busy={toggleBusy === selected.id}
                  >
                    {toggleBusy === selected.id ? (
                      <Spinner />
                    ) : selected.enabled ? (
                      <PowerOff size={13} aria-hidden="true" />
                    ) : (
                      <Power size={13} aria-hidden="true" />
                    )}
                    {selected.enabled ? t("providers.disable") : t("providers.enable")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => {
                      setRemoveError(null);
                      setRemoveTarget(selected);
                    }}
                    disabled={!selected.custom}
                    title={
                      selected.custom
                        ? t("providers.delete")
                        : t("providers.delete_integrated_hint")
                    }
                  >
                    <Trash2 size={13} aria-hidden="true" />
                    {t("providers.delete")}
                  </button>
                </div>
              </header>

              {!selected.enabled ? (
                <p className="muted small">{t("models.provider_disabled_hint")}</p>
              ) : null}

              <dl className="provider-facts">
                <div>
                  <dt>{t("providers.field_kind")}</dt>
                  <dd>{t(`models.kind_${selected.kind}`)}</dd>
                </div>
                <div>
                  <dt>{t("providers.field_endpoint")}</dt>
                  <dd className="mono">{selected.endpoint_path ?? "—"}</dd>
                </div>
                <div>
                  <dt>{t("providers.field_timeout")}</dt>
                  <dd>{t("providers.timeout_value", { secs: selected.timeout_secs })}</dd>
                </div>
                <div>
                  <dt>{t("providers.field_streaming")}</dt>
                  <dd>
                    {selected.streaming
                      ? t("providers.streaming_on")
                      : t("providers.streaming_off")}
                  </dd>
                </div>
                <div>
                  <dt>{t("providers.field_default_model")}</dt>
                  <dd className="mono">
                    {selected.default_model
                      ? t("providers.default_model_value", { model: selected.default_model })
                      : t("providers.default_model_none")}
                  </dd>
                </div>
                <div>
                  <dt>{t("providers.field_api_key")}</dt>
                  <dd>
                    {keyStored ? t("providers.key_stored") : t("providers.key_none")}
                    {!requiresSecret ? ` · ${t("models.local_hint")}` : ""}
                  </dd>
                </div>
              </dl>

              {storedHeaders.length > 0 ? (
                <details className="provider-headers">
                  <summary>{t("providers.headers_value", { count: storedHeaders.length })}</summary>
                  <ul className="headers-preview">
                    {storedHeaders.map((entry) => (
                      <li key={entry.name}>
                        <code>{entry.name}</code>
                      </li>
                    ))}
                  </ul>
                </details>
              ) : null}

              {info.error ? <ErrorBanner error={info.error} /> : null}
              {info.message ? <Notice tone="success">{info.message}</Notice> : null}
              {state.error ? <ErrorBanner error={state.error} /> : null}

              {state.result ? (
                <Notice
                  tone={connectionNoticeTone(state.result.status)}
                  title={t(connectionStatusKey(state.result.status))}
                >
                  <div className="inline-status">
                    <Badge tone={connectionStatusTone(state.result.status)}>
                      {t(connectionStatusKey(state.result.status))}
                    </Badge>
                    <span className="small muted">
                      {t("providers.test_latency", { ms: state.result.latency_ms })}
                    </span>
                    <span className="small muted">
                      {t("providers.test_models", { count: state.result.models })}
                    </span>
                    <span className="small muted">
                      {t("providers.test_protocol", { protocol: state.result.protocol })}
                    </span>
                  </div>
                  <p className="mono small provider-raw-detail">
                    {t("providers.test_detail", { detail: state.result.detail })}
                  </p>
                </Notice>
              ) : null}

              <section className="provider-detail-models" aria-labelledby="provider-models-title">
                <div className="section-heading">
                  <h3 id="provider-models-title">
                    {t("providers.models_section", { count: selected.models.length })}
                  </h3>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => setModelsFor(selected.id)}
                  >
                    <Boxes size={13} aria-hidden="true" />
                    {t("providers.manage_models")}
                  </button>
                </div>
                {selected.models.length === 0 ? (
                  <p className="muted small">{t("providers.models_empty_description")}</p>
                ) : (
                  <ul className="provider-model-list">
                    {selected.models.map((entry) => {
                      const isDefault = entry.id === selected.default_model;
                      const busy = modelBusy === `${selected.id}:${entry.id}`;
                      return (
                        <li key={entry.id} className="provider-model-row">
                          <span className="mono small provider-model-id">{entry.id}</span>
                          <span className="small muted provider-model-label">
                            {entry.label && entry.label !== entry.id ? entry.label : ""}
                          </span>
                          {isDefault ? (
                            <Badge tone="accent">{t("models.default_badge")}</Badge>
                          ) : null}
                          <div className="button-row compact provider-model-actions">
                            <button
                              type="button"
                              className="ghost small"
                              onClick={() => void setDefaultModel(selected, entry.id)}
                              disabled={busy || isDefault}
                              aria-busy={busy}
                              title={t("providers.set_default")}
                            >
                              {busy ? <Spinner /> : <Star size={12} aria-hidden="true" />}
                              {t("providers.set_default")}
                            </button>
                            <button
                              type="button"
                              className="ghost small"
                              onClick={() => setModelToRemove({ provider: selected, modelId: entry.id })}
                              disabled={busy}
                              title={t("providers.model_remove")}
                            >
                              <Trash2 size={12} aria-hidden="true" />
                              {t("providers.model_remove")}
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>

              {!selected.custom ? (
                <p className="field-hint">{t("providers.delete_integrated_hint")}</p>
              ) : null}
            </section>
          ) : (
            <section className="provider-detail card">
              <EmptyState
                icon={<Server size={22} aria-hidden="true" />}
                title={t("providers.detail_empty_title")}
                description={<p>{t("providers.detail_empty_description")}</p>}
              />
            </section>
          )}
        </div>
      )}

      <ProviderFormModal
        open={formOpen}
        provider={formProvider}
        onClose={() => {
          setFormOpen(false);
          setFormProvider(null);
        }}
        onSaved={(saved) => void handleSaved(saved)}
      />

      <ProviderModelsModal
        open={modelsFor !== null}
        provider={modelsProvider}
        onClose={() => setModelsFor(null)}
        onUpdated={handleModelUpdate}
        onRefresh={refreshModels}
      />

      <ConfirmDialog
        open={removeTarget !== null}
        title={t("providers.delete_title")}
        description={t("providers.delete_description", { label: removeTarget?.label ?? "" })}
        confirmLabel={t("providers.delete")}
        danger
        busy={removeBusy}
        onConfirm={() => void confirmDelete()}
        onCancel={() => {
          setRemoveTarget(null);
          setRemoveError(null);
        }}
      >
        {removeError ? <ErrorBanner error={removeError} /> : null}
        {removeTarget && !removeTarget.custom ? (
          <p className="field-error">{t("providers.delete_integrated_hint")}</p>
        ) : null}
      </ConfirmDialog>

      <ConfirmDialog
        open={modelToRemove !== null}
        title={t("providers.model_remove_title")}
        description={
          <p>
            {t("providers.model_remove_confirm", {
              model: modelToRemove?.modelId ?? "",
            })}
          </p>
        }
        confirmLabel={t("providers.model_confirm_yes")}
        danger
        busy={modelBusy !== null}
        onConfirm={() => void confirmRemoveModel()}
        onCancel={() => setModelToRemove(null)}
      />
    </>
  );
}
