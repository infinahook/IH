import { useState } from "react";
import {
  Boxes,
  KeyRound,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Server,
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

export function ProvidersPage() {
  const { t, config, refreshConfig } = useApp();
  const providers = useAsync(() => api.providers.list(), []);
  const [tests, setTests] = useState<Record<string, TestState>>({});
  const [feedback, setFeedback] = useState<Record<string, Feedback>>({});
  const [toggleBusy, setToggleBusy] = useState<string | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [formProvider, setFormProvider] = useState<ProviderView | null>(null);
  const [modelsFor, setModelsFor] = useState<string | null>(null);
  const [removeTarget, setRemoveTarget] = useState<ProviderView | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);
  const [removeError, setRemoveError] = useState<AppError | null>(null);
  const [pageMessage, setPageMessage] = useState<string | null>(null);

  const list = sortProviders(providers.data ?? []);
  const modelsProvider = providers.data?.find((entry) => entry.id === modelsFor) ?? null;

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
      await reload();
    } catch (reason) {
      setRemoveError(toAppError(reason));
    } finally {
      setRemoveBusy(false);
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
        <div className="provider-grid">
          {list.map((provider) => {
            const state = testFor(provider.id);
            const info = feedbackFor(provider.id);
            const requiresSecret = providerRequiresSecret(provider.kind);
            const keyStored = provider.secret_stored || provider.has_secret;
            const missingKey = requiresSecret && !keyStored;
            const busy = toggleBusy === provider.id;
            const cannotTest = !provider.enabled || missingKey || state.busy;
            const storedHeaders = headerList(
              config?.providers.find((entry) => entry.id === provider.id)?.headers,
            );

            return (
              <article className="card provider-card" key={provider.id}>
                <header className="provider-header">
                  <div>
                    <div className="provider-title">
                      <h2>{provider.label}</h2>
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
                      <Badge tone={keyStored ? "success" : "neutral"}>
                        <KeyRound size={11} aria-hidden="true" />
                        {keyStored ? t("providers.key_stored") : t("providers.key_none")}
                      </Badge>
                    </div>
                    <p className="mono small provider-url" title={provider.base_url}>
                      {provider.base_url}
                    </p>
                  </div>
                  <Badge tone="neutral">{t(`models.kind_${provider.kind}`)}</Badge>
                </header>

                {!provider.enabled ? (
                  <p className="muted small">{t("models.provider_disabled_hint")}</p>
                ) : null}

                <div className="provider-meta">
                  <span>
                    <Boxes size={12} aria-hidden="true" />
                    {t("providers.models_count", { count: provider.models.length })}
                  </span>
                  <span>
                    {provider.default_model
                      ? t("providers.default_model_value", { model: provider.default_model })
                      : t("providers.default_model_none")}
                  </span>
                </div>

                <div className="inline-status provider-flags">
                  {provider.endpoint_path ? (
                    <Badge tone="neutral" title={provider.endpoint_path}>
                      {t("providers.endpoint_path_value", { path: provider.endpoint_path })}
                    </Badge>
                  ) : null}
                  <Badge tone="neutral">
                    {provider.streaming
                      ? t("providers.streaming_on")
                      : t("providers.streaming_off")}
                  </Badge>
                  <Badge tone="neutral">
                    {t("providers.timeout_value", { secs: provider.timeout_secs })}
                  </Badge>
                </div>

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

                <div className="button-row">
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => openEdit(provider)}
                  >
                    <Pencil size={13} aria-hidden="true" />
                    {t("providers.edit")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void testConnection(provider)}
                    disabled={cannotTest}
                    aria-busy={state.busy}
                    title={
                      !provider.enabled
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
                    onClick={() => setModelsFor(provider.id)}
                  >
                    <Boxes size={13} aria-hidden="true" />
                    {t("providers.manage_models")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void toggleEnabled(provider)}
                    disabled={busy}
                    aria-busy={busy}
                  >
                    {busy ? (
                      <Spinner />
                    ) : provider.enabled ? (
                      <PowerOff size={13} aria-hidden="true" />
                    ) : (
                      <Power size={13} aria-hidden="true" />
                    )}
                    {provider.enabled ? t("providers.disable") : t("providers.enable")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => {
                      setRemoveError(null);
                      setRemoveTarget(provider);
                    }}
                    disabled={!provider.custom}
                    title={
                      provider.custom ? t("providers.delete") : t("providers.delete_integrated_hint")
                    }
                  >
                    <Trash2 size={13} aria-hidden="true" />
                    {t("providers.delete")}
                  </button>
                </div>

                {!provider.custom ? (
                  <p className="field-hint">{t("providers.delete_integrated_hint")}</p>
                ) : null}
              </article>
            );
          })}
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
    </>
  );
}
