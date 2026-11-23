import { useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronRight,
  Eye,
  EyeOff,
  KeyRound,
  Plus,
  RefreshCw,
  Star,
  Trash2,
  Wifi,
  X,
} from "lucide-react";
import {
  api,
  toAppError,
  AppError,
  type ConnectionTest,
  type ProviderConfig,
  type ProviderInput,
  type ProviderView,
} from "../lib/api";
import {
  baseUrlError,
  connectionNoticeTone,
  connectionStatusKey,
  providerRequiresSecret,
  serializeHeaders,
  sortProviders,
} from "../lib/providers";
import { useApp } from "../lib/store";
import {
  PROVIDER_DRAWER_SECTIONS,
  drawerSectionLabelKey,
  type ProviderDrawerSection,
  type ProviderDrawerState,
} from "../lib/ux";
import { Badge } from "./Badge";
import { Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";
import { Notice } from "./Notice";

interface Draft {
  base_url: string;
  endpoint_path: string;
  timeout_secs: string;
  streaming: boolean;
  temperature: string;
  max_tokens: string;
  headers: string;
}

function draftFrom(
  provider: ProviderView | null,
  stored: ProviderConfig | null,
): Draft {
  return {
    base_url: provider?.base_url ?? "",
    endpoint_path: provider?.endpoint_path ?? "",
    timeout_secs: String(provider?.timeout_secs ?? stored?.timeout_secs ?? 120),
    streaming: provider?.streaming ?? stored?.streaming ?? true,
    temperature: stored?.temperature != null ? String(stored.temperature) : "",
    max_tokens: stored?.max_tokens != null ? String(stored.max_tokens) : "",
    headers: serializeHeaders(stored?.headers),
  };
}

function parseHeadersSafe(text: string): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line.length === 0) {
      continue;
    }
    const separator = line.indexOf(":");
    if (separator <= 0) {
      continue;
    }
    headers[line.slice(0, separator).trim()] = line.slice(separator + 1).trim();
  }
  return headers;
}

function headersValid(text: string): boolean {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .every((line) => {
      const separator = line.indexOf(":");
      return separator > 0 && line.slice(0, separator).trim().length > 0;
    });
}

/**
 * Panneau de configuration secondaire de la conversation.
 *
 * C'est un drawer latéral : la conversation reste montée et un streaming en
 * cours continue derrière. Chaque section est repliable et n'appelle que les
 * commandes existantes du noyau.
 */
export function ProviderConfigDrawer({
  state,
  providers,
  selectedProviderId,
  selectedModel,
  onClose,
  onSelectProvider,
  onSelectModel,
  onToggleSection,
  onSelectSection,
  onReload,
}: {
  state: ProviderDrawerState;
  providers: readonly ProviderView[];
  selectedProviderId: string;
  selectedModel: string;
  onClose: () => void;
  onSelectProvider: (providerId: string) => void;
  onSelectModel: (model: string) => void;
  onToggleSection: (section: ProviderDrawerSection) => void;
  onSelectSection: (section: ProviderDrawerSection) => void;
  onReload: () => Promise<void>;
}) {
  const { t, config, refreshConfig, secretStatus } = useApp();
  const closeRef = useRef<HTMLButtonElement>(null);
  const [draft, setDraft] = useState<Draft>(() => draftFrom(null, null));
  const [busy, setBusy] = useState<string | null>(null);
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [message, setMessage] = useState<{ tone: "success" | "warning"; text: string } | null>(
    null,
  );
  const [testResult, setTestResult] = useState<ConnectionTest | null>(null);
  const [secretInput, setSecretInput] = useState("");
  const [secretVisible, setSecretVisible] = useState(false);
  const [manualId, setManualId] = useState("");
  const [manualLabel, setManualLabel] = useState("");
  const [pendingRemove, setPendingRemove] = useState<string | null>(null);

  const ordered = useMemo(() => sortProviders([...providers]), [providers]);
  const selected = useMemo(
    () => providers.find((entry) => entry.id === selectedProviderId) ?? providers[0] ?? null,
    [providers, selectedProviderId],
  );
  const stored = useMemo(
    () => config?.providers.find((entry) => entry.id === selected?.id) ?? null,
    [config, selected?.id],
  );
  const requiresSecret = selected ? providerRequiresSecret(selected.kind) : false;
  const storedKey = selected !== null && (selected.has_secret || selected.secret_stored);

  useEffect(() => {
    if (!state.open) {
      return undefined;
    }
    closeRef.current?.focus();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [state.open, onClose]);

  // Un changement de fournisseur réinitialise le brouillon, jamais la conversation.
  useEffect(() => {
    if (!state.open) {
      return;
    }
    setDraft(draftFrom(selected, stored));
    setTestResult(null);
    setActionError(null);
    setMessage(null);
    setSecretInput("");
    setManualId("");
    setManualLabel("");
    setPendingRemove(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.open, selected?.id]);

  if (!state.open) {
    return null;
  }

  const run = async (kind: string, task: () => Promise<void>) => {
    setBusy(kind);
    setActionError(null);
    setMessage(null);
    try {
      await task();
      await onReload();
      await refreshConfig();
    } catch (reason) {
      setActionError(toAppError(reason));
    } finally {
      setBusy(null);
    }
  };

  const patch = (update: Partial<Draft>) => {
    setDraft((current) => ({ ...current, ...update }));
    setActionError(null);
    setMessage(null);
  };

  const validateDraft = (): string | null => {
    const urlError = baseUrlError(draft.base_url);
    if (urlError) {
      return t(`providers.url_error_${urlError}`);
    }
    const timeout = Number(draft.timeout_secs);
    if (!Number.isInteger(timeout) || timeout < 1 || timeout > 3600) {
      return t("providers.timeout_error");
    }
    if (draft.temperature.trim().length > 0) {
      const temperature = Number(draft.temperature);
      if (!Number.isFinite(temperature) || temperature < 0 || temperature > 2) {
        return t("providers.temperature_error");
      }
    }
    if (draft.max_tokens.trim().length > 0) {
      const tokens = Number(draft.max_tokens);
      if (!Number.isInteger(tokens) || tokens <= 0) {
        return t("providers.max_tokens_error");
      }
    }
    if (!headersValid(draft.headers)) {
      return t("providers.headers_errors");
    }
    return null;
  };

  const saveProvider = () =>
    run("save", async () => {
      if (!selected) {
        return;
      }
      const invalid = validateDraft();
      if (invalid) {
        setActionError(new AppError("invalid_input", invalid, true));
        return;
      }
      const input: ProviderInput = {
        id: selected.id,
        label: selected.label,
        kind: selected.kind,
        base_url: draft.base_url.trim(),
        endpoint_path: draft.endpoint_path.trim() || null,
        headers: parseHeadersSafe(draft.headers),
        timeout_secs: Number(draft.timeout_secs),
        streaming: draft.streaming,
        temperature: draft.temperature.trim() === "" ? null : Number(draft.temperature),
        max_tokens: draft.max_tokens.trim() === "" ? null : Number(draft.max_tokens),
        default_model: selected.default_model,
        // `provider_update` remplace la liste : on renvoie les modèles connus.
        models: selected.models,
        api_key: null,
      };
      await api.providers.update(selected.id, input);
      setMessage({ tone: "success", text: t("chat.config_saved") });
    });

  const setDefaultModel = (modelId: string) =>
    run("default", async () => {
      if (!selected) {
        return;
      }
      await api.providers.setDefaultModel(selected.id, modelId);
      onSelectModel(modelId);
      setMessage({ tone: "success", text: t("providers.model_default_set", { model: modelId }) });
    });

  const removeModel = (modelId: string) =>
    run("remove", async () => {
      if (!selected) {
        return;
      }
      await api.providers.removeModel(selected.id, modelId);
      setPendingRemove(null);
      setMessage({ tone: "success", text: t("providers.model_removed", { model: modelId }) });
    });

  const addModel = () =>
    run("add", async () => {
      if (!selected) {
        return;
      }
      const modelId = manualId.trim();
      if (modelId.length === 0) {
        setActionError(new AppError("invalid_input", t("providers.models_id_required"), true));
        return;
      }
      await api.providers.addModel(selected.id, modelId, manualLabel.trim() || null);
      setManualId("");
      setManualLabel("");
      setMessage({ tone: "success", text: t("providers.model_added", { model: modelId }) });
    });

  const discoverModels = () =>
    run("discover", async () => {
      if (!selected) {
        return;
      }
      const found = await api.providers.models(selected.id);
      setMessage({
        tone: found.length === 0 ? "warning" : "success",
        text:
          found.length === 0
            ? t("providers.models_discovery_empty")
            : t("providers.models_discovery_ok", { count: found.length }),
      });
    });

  const saveSecret = () =>
    run("secret", async () => {
      if (!selected) {
        return;
      }
      const secret = secretInput.trim();
      if (secret.length === 0) {
        setActionError(new AppError("invalid_input", t("models.key_empty"), true));
        return;
      }
      await api.providers.setSecret(selected.id, secret);
      setSecretInput("");
      setMessage({ tone: "success", text: t("models.secret_saved") });
    });

  const clearSecret = () =>
    run("clear", async () => {
      if (!selected) {
        return;
      }
      await api.providers.clearSecret(selected.id);
      setMessage({ tone: "success", text: t("models.secret_cleared") });
    });

  const testConnection = () =>
    run("test", async () => {
      if (!selected) {
        return;
      }
      const result = await api.providers.test(selected.id);
      setTestResult(result);
    });

  const section = (id: ProviderDrawerSection, title: string) => {
    const expanded = state.expanded.includes(id);
    return (
      <section className={expanded ? "drawer-section expanded" : "drawer-section"} key={id}>
        <button
          type="button"
          className="drawer-section-head"
          aria-expanded={expanded}
          aria-controls={`drawer-section-${id}`}
          onClick={() => onToggleSection(id)}
        >
          <ChevronRight size={14} aria-hidden="true" className="drawer-chevron" />
          <span className="drawer-section-title">{title}</span>
        </button>
        {expanded ? (
          <div className="drawer-section-body" id={`drawer-section-${id}`}>
            {id === "providers" ? (
              <>
                <ul className="drawer-provider-list" aria-label={t("chat.section_providers")}>
                  {ordered.map((entry) => {
                    const active = entry.id === selected?.id;
                    const keyMissing =
                      providerRequiresSecret(entry.kind) &&
                      !entry.has_secret &&
                      !entry.secret_stored;
                    return (
                      <li key={entry.id}>
                        <button
                          type="button"
                          className={
                            active
                              ? "drawer-provider-row active"
                              : "drawer-provider-row"
                          }
                          aria-current={active ? "true" : undefined}
                          onClick={() => {
                            onSelectProvider(entry.id);
                            onSelectSection("models");
                          }}
                        >
                          <span
                            className={`status-dot ${
                              !entry.enabled
                                ? "status-dot-off"
                                : keyMissing
                                  ? "status-dot-warn"
                                  : "status-dot-on"
                            }`}
                            aria-hidden="true"
                          />
                          <span className="drawer-provider-main">
                            <span className="drawer-provider-label">{entry.label}</span>
                            <span className="small muted">
                              {t(`models.kind_${entry.kind}`)} ·{" "}
                              {t("providers.models_count", { count: entry.models.length })}
                            </span>
                          </span>
                          {entry.custom ? (
                            <Badge tone="info">{t("providers.badge_custom")}</Badge>
                          ) : null}
                        </button>
                      </li>
                    );
                  })}
                </ul>
                {selected ? (
                  <div className="button-row compact drawer-inline-actions">
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void testConnection()}
                      disabled={
                        busy !== null ||
                        !selected.enabled ||
                        (requiresSecret && !storedKey)
                      }
                      aria-busy={busy === "test"}
                      title={
                        !selected.enabled
                          ? t("providers.test_disabled")
                          : requiresSecret && !storedKey
                            ? t("providers.test_no_secret")
                            : t("providers.test_hint")
                      }
                    >
                      {busy === "test" ? <Spinner /> : <Wifi size={13} aria-hidden="true" />}
                      {t("providers.test")}
                    </button>
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => onSelectSection("advanced")}
                    >
                      {t("chat.section_advanced")}
                    </button>
                  </div>
                ) : (
                  <p className="muted small">{t("chat.no_provider")}</p>
                )}
              </>
            ) : null}

            {id === "models" ? (
              <>
                {selected ? (
                  <>
                    <div className="drawer-models-head">
                      <span className="small muted">
                        {t("providers.models_count", { count: selected.models.length })}
                      </span>
                      <button
                        type="button"
                        className="ghost small"
                        onClick={() => void discoverModels()}
                        disabled={busy !== null}
                        aria-busy={busy === "discover"}
                        title={t("providers.discovery_hint")}
                      >
                        {busy === "discover" ? (
                          <Spinner />
                        ) : (
                          <RefreshCw size={13} aria-hidden="true" />
                        )}
                        {busy === "discover"
                          ? t("providers.models_discovering")
                          : t("providers.models_discover")}
                      </button>
                    </div>
                    {selected.models.length === 0 ? (
                      <p className="muted small">{t("providers.models_empty_description")}</p>
                    ) : (
                      <ul className="drawer-model-list">
                        {selected.models.map((entry) => {
                          const isDefault = entry.id === selected.default_model;
                          const isCurrent = entry.id === selectedModel;
                          return (
                            <li
                              key={entry.id}
                              className={
                                isCurrent ? "drawer-model-row current" : "drawer-model-row"
                              }
                            >
                              <button
                                type="button"
                                className="drawer-model-pick"
                                onClick={() => onSelectModel(entry.id)}
                                title={t("chat.use_model")}
                              >
                                <span className="mono small">{entry.id}</span>
                                {isDefault ? (
                                  <Badge tone="accent">{t("models.default_badge")}</Badge>
                                ) : null}
                              </button>
                              <div className="drawer-model-actions">
                                <button
                                  type="button"
                                  className="icon-button"
                                  onClick={() => void setDefaultModel(entry.id)}
                                  disabled={busy !== null || isDefault}
                                  title={t("providers.set_default")}
                                  aria-label={t("providers.set_default")}
                                >
                                  <Star size={12} aria-hidden="true" />
                                </button>
                                {pendingRemove === entry.id ? (
                                  <>
                                    <button
                                      type="button"
                                      className="danger small"
                                      onClick={() => void removeModel(entry.id)}
                                      disabled={busy !== null}
                                    >
                                      {busy === "remove" ? (
                                        <Spinner />
                                      ) : (
                                        <Trash2 size={12} aria-hidden="true" />
                                      )}
                                      {t("providers.model_confirm_yes")}
                                    </button>
                                    <button
                                      type="button"
                                      className="ghost small"
                                      onClick={() => setPendingRemove(null)}
                                    >
                                      {t("common.cancel")}
                                    </button>
                                  </>
                                ) : (
                                  <button
                                    type="button"
                                    className="icon-button"
                                    onClick={() => setPendingRemove(entry.id)}
                                    disabled={busy !== null}
                                    title={t("providers.model_remove")}
                                    aria-label={t("providers.model_remove")}
                                  >
                                    <Trash2 size={12} aria-hidden="true" />
                                  </button>
                                )}
                              </div>
                            </li>
                          );
                        })}
                      </ul>
                    )}
                    <div className="drawer-manual-model">
                      <p className="field-label">{t("providers.models_add_manual")}</p>
                      <div className="drawer-manual-row">
                        <input
                          className="input mono"
                          aria-label={t("providers.model_id")}
                          placeholder={t("providers.model_id")}
                          value={manualId}
                          disabled={busy !== null}
                          onChange={(event) => setManualId(event.target.value)}
                        />
                        <input
                          className="input"
                          aria-label={t("providers.model_label")}
                          placeholder={t("providers.model_label")}
                          value={manualLabel}
                          disabled={busy !== null}
                          onChange={(event) => setManualLabel(event.target.value)}
                        />
                        <button
                          type="button"
                          className="ghost small"
                          onClick={() => void addModel()}
                          disabled={busy !== null || manualId.trim().length === 0}
                          aria-busy={busy === "add"}
                        >
                          {busy === "add" ? <Spinner /> : <Plus size={13} aria-hidden="true" />}
                          {t("providers.model_add")}
                        </button>
                      </div>
                    </div>
                  </>
                ) : (
                  <p className="muted small">{t("chat.no_provider")}</p>
                )}
              </>
            ) : null}

            {id === "endpoints" ? (
              <>
                {selected ? (
                  <>
                    <label className="drawer-field">
                      <span className="field-label">{t("providers.field_base_url")}</span>
                      <input
                        className="input mono"
                        value={draft.base_url}
                        disabled={busy !== null}
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(event) => patch({ base_url: event.target.value })}
                      />
                    </label>
                    <label className="drawer-field">
                      <span className="field-label">{t("providers.field_endpoint")}</span>
                      <input
                        className="input mono"
                        value={draft.endpoint_path}
                        placeholder="/v1/chat/completions"
                        disabled={busy !== null}
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(event) => patch({ endpoint_path: event.target.value })}
                      />
                    </label>
                    <p className="field-hint">{t("chat.endpoints_hint")}</p>
                    <button
                      type="button"
                      className="primary small"
                      onClick={() => void saveProvider()}
                      disabled={busy !== null}
                      aria-busy={busy === "save"}
                    >
                      {busy === "save" ? <Spinner /> : null}
                      {t("common.save")}
                    </button>
                  </>
                ) : (
                  <p className="muted small">{t("chat.no_provider")}</p>
                )}
              </>
            ) : null}

            {id === "keys" ? (
              <>
                {selected ? (
                  <>
                    <div className="inline-status">
                      <Badge tone={storedKey ? "success" : requiresSecret ? "warning" : "neutral"}>
                        <KeyRound size={11} aria-hidden="true" />
                        {storedKey ? t("providers.key_stored") : t("providers.key_none")}
                      </Badge>
                      {!requiresSecret ? (
                        <span className="small muted">{t("models.local_hint")}</span>
                      ) : null}
                    </div>
                    {requiresSecret ? (
                      <>
                        <label className="drawer-field">
                          <span className="field-label">{t("providers.field_api_key")}</span>
                          <div className="drawer-secret-row">
                            <input
                              className="input mono"
                              type={secretVisible ? "text" : "password"}
                              autoComplete="off"
                              spellCheck={false}
                              value={secretInput}
                              placeholder={t("providers.field_api_key_placeholder")}
                              disabled={busy !== null}
                              onChange={(event) => setSecretInput(event.target.value)}
                            />
                            <button
                              type="button"
                              className="icon-button"
                              onClick={() => setSecretVisible((value) => !value)}
                              title={secretVisible ? t("chat.hide_secret") : t("chat.show_secret")}
                              aria-label={
                                secretVisible ? t("chat.hide_secret") : t("chat.show_secret")
                              }
                            >
                              {secretVisible ? (
                                <EyeOff size={14} aria-hidden="true" />
                              ) : (
                                <Eye size={14} aria-hidden="true" />
                              )}
                            </button>
                          </div>
                        </label>
                        <div className="button-row compact">
                          <button
                            type="button"
                            className="primary small"
                            onClick={() => void saveSecret()}
                            disabled={busy !== null || secretInput.trim().length === 0}
                            aria-busy={busy === "secret"}
                          >
                            {busy === "secret" ? <Spinner /> : null}
                            {t("chat.save_key")}
                          </button>
                          <button
                            type="button"
                            className="ghost small"
                            onClick={() => void clearSecret()}
                            disabled={busy !== null || !storedKey}
                            aria-busy={busy === "clear"}
                          >
                            {busy === "clear" ? <Spinner /> : <Trash2 size={12} aria-hidden="true" />}
                            {t("chat.clear_key")}
                          </button>
                        </div>
                        {secretStatus?.state === "unavailable" ? (
                          <p className="field-hint">{t("models.secret_unavailable_hint")}</p>
                        ) : null}
                      </>
                    ) : (
                      <p className="muted small">{t("chat.local_provider_no_key")}</p>
                    )}
                  </>
                ) : (
                  <p className="muted small">{t("chat.no_provider")}</p>
                )}
              </>
            ) : null}

            {id === "advanced" ? (
              <>
                {selected ? (
                  <>
                    <div className="drawer-two-cols">
                      <label className="drawer-field">
                        <span className="field-label">{t("providers.field_timeout")}</span>
                        <input
                          className="input"
                          type="number"
                          min={1}
                          max={3600}
                          value={draft.timeout_secs}
                          disabled={busy !== null}
                          onChange={(event) => patch({ timeout_secs: event.target.value })}
                        />
                      </label>
                      <label className="drawer-field">
                        <span className="field-label">{t("providers.field_temperature")}</span>
                        <input
                          className="input"
                          type="number"
                          min={0}
                          max={2}
                          step={0.1}
                          placeholder="—"
                          value={draft.temperature}
                          disabled={busy !== null}
                          onChange={(event) => patch({ temperature: event.target.value })}
                        />
                      </label>
                      <label className="drawer-field">
                        <span className="field-label">{t("providers.field_max_tokens")}</span>
                        <input
                          className="input"
                          type="number"
                          min={1}
                          placeholder="—"
                          value={draft.max_tokens}
                          disabled={busy !== null}
                          onChange={(event) => patch({ max_tokens: event.target.value })}
                        />
                      </label>
                      <label className="checkbox-row drawer-checkbox">
                        <input
                          type="checkbox"
                          checked={draft.streaming}
                          disabled={busy !== null}
                          onChange={(event) => patch({ streaming: event.target.checked })}
                        />
                        <span>{t("providers.field_streaming_hint")}</span>
                      </label>
                    </div>
                    <label className="drawer-field">
                      <span className="field-label">{t("providers.field_headers")}</span>
                      <textarea
                        className="input mono"
                        rows={4}
                        spellCheck={false}
                        value={draft.headers}
                        placeholder={t("providers.field_headers_placeholder")}
                        disabled={busy !== null}
                        onChange={(event) => patch({ headers: event.target.value })}
                      />
                    </label>
                    <p className="field-hint">{t("chat.advanced_hint")}</p>
                    <button
                      type="button"
                      className="primary small"
                      onClick={() => void saveProvider()}
                      disabled={busy !== null}
                      aria-busy={busy === "save"}
                    >
                      {busy === "save" ? <Spinner /> : null}
                      {t("common.save")}
                    </button>
                  </>
                ) : (
                  <p className="muted small">{t("chat.no_provider")}</p>
                )}
              </>
            ) : null}
          </div>
        ) : null}
      </section>
    );
  };

  return (
    <div
      className="provider-drawer-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <aside
        className="provider-drawer"
        role="dialog"
        aria-modal="false"
        aria-labelledby="provider-drawer-title"
      >
        <header className="provider-drawer-head">
          <div>
            <h2 id="provider-drawer-title">{t("chat.config_title")}</h2>
            <p className="small muted">{t("chat.config_subtitle")}</p>
          </div>
          <button
            ref={closeRef}
            type="button"
            className="icon-button"
            onClick={onClose}
            aria-label={t("chat.close_config")}
            title={t("chat.close_config")}
          >
            <X size={15} aria-hidden="true" />
          </button>
        </header>

        <div className="provider-drawer-body">
          {actionError ? (
            <ErrorBanner error={actionError} onDismiss={() => setActionError(null)} />
          ) : null}
          {message ? <Notice tone={message.tone}>{message.text}</Notice> : null}
          {testResult ? (
            <Notice
              tone={connectionNoticeTone(testResult.status)}
              title={t(connectionStatusKey(testResult.status))}
            >
              <div className="inline-status">
                <Badge tone={testResult.status === "ok" ? "success" : "warning"}>
                  {t(connectionStatusKey(testResult.status))}
                </Badge>
                <span className="small muted">
                  {t("providers.test_latency", { ms: testResult.latency_ms })}
                </span>
                <span className="small muted">
                  {t("providers.test_models", { count: testResult.models })}
                </span>
              </div>
            </Notice>
          ) : null}

          {selected ? (
            <div className="drawer-current">
              <span className="small muted">{t("chat.current_provider")}</span>
              <strong>{selected.label}</strong>
              {!selected.enabled ? (
                <Badge tone="danger">{t("providers.badge_inactive")}</Badge>
              ) : null}
              {secretStatus?.state === "unavailable" && requiresSecret ? (
                <Badge tone="warning">{t("models.secret_store")}</Badge>
              ) : null}
            </div>
          ) : (
            <p className="muted small">{t("chat.no_provider")}</p>
          )}

          {PROVIDER_DRAWER_SECTIONS.map((id) => section(id, t(drawerSectionLabelKey(id))))}
        </div>
      </aside>
    </div>
  );
}
