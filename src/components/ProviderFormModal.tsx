import { useEffect, useMemo, useState } from "react";
import { Plus, Save } from "lucide-react";
import {
  api,
  toAppError,
  type AppError,
  type ProviderConfig,
  type ProviderInput,
  type ProviderKind,
  type ProviderView,
} from "../lib/api";
import {
  PROVIDER_KINDS,
  baseUrlError,
  headerList,
  parseHeaders,
  providerSlugError,
  serializeHeaders,
} from "../lib/providers";
import { useApp } from "../lib/store";
import { ErrorBanner } from "./ErrorBanner";
import { Field } from "./Field";
import { Modal } from "./Modal";

const BASE_URL_PLACEHOLDERS: Record<ProviderKind, string> = {
  openai_compatible: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com",
  google: "https://generativelanguage.googleapis.com",
  ollama: "http://localhost:11434",
};

interface FormState {
  id: string;
  label: string;
  kind: ProviderKind;
  base_url: string;
  endpoint_path: string;
  timeout_secs: string;
  streaming: boolean;
  temperature: string;
  max_tokens: string;
  default_model: string;
  api_key: string;
  headers: string;
}

function initialState(provider: ProviderView | null): FormState {
  return {
    id: provider?.id ?? "",
    label: provider?.label ?? "",
    kind: provider?.kind ?? "openai_compatible",
    base_url: provider?.base_url ?? "",
    endpoint_path: provider?.endpoint_path ?? "",
    timeout_secs: String(provider?.timeout_secs ?? 120),
    streaming: provider?.streaming ?? true,
    temperature: "",
    max_tokens: "",
    default_model: provider?.default_model ?? "",
    api_key: "",
    headers: "",
  };
}

function initialStateFromConfig(
  provider: ProviderView | null,
  stored: ProviderConfig | null,
): FormState {
  if (!provider) {
    return initialState(null);
  }
  const base = initialState(provider);
  return {
    ...base,
    timeout_secs: String(provider.timeout_secs ?? stored?.timeout_secs ?? 120),
    streaming: provider.streaming ?? stored?.streaming ?? true,
    temperature: stored?.temperature != null ? String(stored.temperature) : "",
    max_tokens: stored?.max_tokens != null ? String(stored.max_tokens) : "",
    headers: serializeHeaders(stored?.headers),
  };
}

export function ProviderFormModal({
  open,
  provider,
  onClose,
  onSaved,
}: {
  open: boolean;
  /** `null` crée un fournisseur ; sinon édition de ce fournisseur. */
  provider: ProviderView | null;
  onClose: () => void;
  onSaved: (saved: ProviderView) => void;
}) {
  const { t, config } = useApp();
  const editing = provider !== null;
  // `providers_list` ne renvoie ni les en-têtes, ni la température, ni max_tokens :
  // ces valeurs viennent de `config_get` pour ne pas les effacer à l'édition.
  const stored =
    provider !== null ? config?.providers.find((entry) => entry.id === provider.id) ?? null : null;
  const [form, setForm] = useState<FormState>(() =>
    initialStateFromConfig(provider, stored),
  );
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [actionError, setActionError] = useState<AppError | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    setForm(initialStateFromConfig(provider, stored));
    setErrors({});
    setActionError(null);
    // L'ouverture (ou le fournisseur ciblé) réinitialise le formulaire.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, provider?.id]);

  const parsed = useMemo(() => parseHeaders(form.headers), [form.headers]);
  const preview = useMemo(() => headerList(parsed.headers), [parsed.headers]);

  const patch = (update: Partial<FormState>) => {
    setForm((current) => ({ ...current, ...update }));
    setErrors({});
    setActionError(null);
  };

  const validate = (): Record<string, string> => {
    const found: Record<string, string> = {};
    if (!editing) {
      const slugError = providerSlugError(form.id);
      if (slugError) {
        found.id = t(`providers.slug_error_${slugError}`);
      }
    }
    if (form.label.trim().length === 0) {
      found.label = t("providers.label_required");
    }
    const urlError = baseUrlError(form.base_url);
    if (urlError) {
      found.base_url = t(`providers.url_error_${urlError}`);
    }
    const timeout = Number(form.timeout_secs);
    if (!Number.isInteger(timeout) || timeout < 1 || timeout > 3600) {
      found.timeout_secs = t("providers.timeout_error");
    }
    if (form.temperature.trim().length > 0) {
      const temperature = Number(form.temperature);
      if (!Number.isFinite(temperature) || temperature < 0 || temperature > 2) {
        found.temperature = t("providers.temperature_error");
      }
    }
    if (form.max_tokens.trim().length > 0) {
      const maxTokens = Number(form.max_tokens);
      if (!Number.isInteger(maxTokens) || maxTokens <= 0) {
        found.max_tokens = t("providers.max_tokens_error");
      }
    }
    if (parsed.issues.length > 0) {
      found.headers = t("providers.headers_errors");
    }
    return found;
  };

  const submit = async () => {
    const found = validate();
    setErrors(found);
    if (Object.keys(found).length > 0) {
      return;
    }
    const input: ProviderInput = {
      id: form.id.trim(),
      label: form.label.trim(),
      kind: form.kind,
      base_url: form.base_url.trim(),
      endpoint_path: form.endpoint_path.trim() || null,
      headers: parsed.headers,
      timeout_secs: Number(form.timeout_secs),
      streaming: form.streaming,
      temperature: form.temperature.trim() === "" ? null : Number(form.temperature),
      max_tokens: form.max_tokens.trim() === "" ? null : Number(form.max_tokens),
      default_model: form.default_model.trim() || null,
      // `provider_update` remplace la liste : on renvoie les modèles connus pour
      // qu'une simple édition de champ ne les efface pas.
      models: provider?.models ?? [],
      api_key: form.api_key.trim() || null,
    };
    setSaving(true);
    setActionError(null);
    try {
      const saved = editing
        ? await api.providers.update(provider.id, input)
        : await api.providers.create(input);
      onSaved(saved);
    } catch (reason) {
      setActionError(toAppError(reason));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title={
        editing
          ? t("providers.form_edit_title", { label: provider.label })
          : t("providers.form_create_title")
      }
      onClose={onClose}
      size="wide"
      footer={
        <>
          <button type="button" className="ghost" onClick={onClose} disabled={saving}>
            {t("common.cancel")}
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => void submit()}
            disabled={saving}
            aria-busy={saving}
          >
            {editing ? <Save size={14} aria-hidden="true" /> : <Plus size={14} aria-hidden="true" />}
            {editing ? t("providers.save") : t("providers.create")}
          </button>
        </>
      }
    >
      {actionError ? <ErrorBanner error={actionError} onDismiss={() => setActionError(null)} /> : null}

      <div className="form-grid">
        <Field
          label={t("providers.field_id")}
          htmlFor="provider-form-id"
          hint={t("providers.field_id_hint")}
          error={errors.id}
          required={!editing}
        >
          <input
            id="provider-form-id"
            className="input mono"
            value={form.id}
            disabled={editing || saving}
            autoComplete="off"
            spellCheck={false}
            placeholder="mon-fournisseur"
            onChange={(event) => patch({ id: event.target.value })}
          />
        </Field>
        <Field
          label={t("providers.field_label")}
          htmlFor="provider-form-label"
          error={errors.label}
          required
        >
          <input
            id="provider-form-label"
            className="input"
            value={form.label}
            disabled={saving}
            onChange={(event) => patch({ label: event.target.value })}
          />
        </Field>
        <Field label={t("providers.field_kind")} htmlFor="provider-form-kind" required>
          <select
            id="provider-form-kind"
            className="input"
            value={form.kind}
            disabled={saving}
            onChange={(event) => patch({ kind: event.target.value as ProviderKind })}
          >
            {PROVIDER_KINDS.map((kind) => (
              <option key={kind} value={kind}>
                {t(`models.kind_${kind}`)}
              </option>
            ))}
          </select>
        </Field>
        <Field
          label={t("providers.field_base_url")}
          htmlFor="provider-form-base-url"
          hint={t("providers.field_base_url_hint")}
          error={errors.base_url}
          required
        >
          <input
            id="provider-form-base-url"
            className="input mono"
            value={form.base_url}
            disabled={saving}
            autoComplete="off"
            spellCheck={false}
            placeholder={BASE_URL_PLACEHOLDERS[form.kind]}
            onChange={(event) => patch({ base_url: event.target.value })}
          />
        </Field>
        <Field
          label={t("providers.field_endpoint")}
          htmlFor="provider-form-endpoint"
          hint={t("providers.field_endpoint_hint")}
        >
          <input
            id="provider-form-endpoint"
            className="input mono"
            value={form.endpoint_path}
            disabled={saving}
            autoComplete="off"
            spellCheck={false}
            placeholder="/v1/chat/completions"
            onChange={(event) => patch({ endpoint_path: event.target.value })}
          />
        </Field>
        <Field
          label={t("providers.field_timeout")}
          htmlFor="provider-form-timeout"
          error={errors.timeout_secs}
        >
          <input
            id="provider-form-timeout"
            className="input"
            type="number"
            min={1}
            max={3600}
            value={form.timeout_secs}
            disabled={saving}
            onChange={(event) => patch({ timeout_secs: event.target.value })}
          />
        </Field>
        <Field
          label={t("providers.field_temperature")}
          htmlFor="provider-form-temperature"
          hint={t("providers.field_temperature_hint")}
          error={errors.temperature}
        >
          <input
            id="provider-form-temperature"
            className="input"
            type="number"
            min={0}
            max={2}
            step={0.1}
            value={form.temperature}
            disabled={saving}
            placeholder="—"
            onChange={(event) => patch({ temperature: event.target.value })}
          />
        </Field>
        <Field
          label={t("providers.field_max_tokens")}
          htmlFor="provider-form-max-tokens"
          hint={t("providers.field_max_tokens_hint")}
          error={errors.max_tokens}
        >
          <input
            id="provider-form-max-tokens"
            className="input"
            type="number"
            min={1}
            step={1}
            value={form.max_tokens}
            disabled={saving}
            placeholder="—"
            onChange={(event) => patch({ max_tokens: event.target.value })}
          />
        </Field>
        <Field
          label={t("providers.field_default_model")}
          htmlFor="provider-form-default-model"
          hint={t("providers.field_default_model_hint")}
        >
          <input
            id="provider-form-default-model"
            className="input mono"
            list="provider-form-model-list"
            value={form.default_model}
            disabled={saving}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => patch({ default_model: event.target.value })}
          />
          <datalist id="provider-form-model-list">
            {(provider?.models ?? []).map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.label || entry.id}
              </option>
            ))}
          </datalist>
        </Field>
        <div className="field">
          <span className="field-label">{t("providers.field_streaming")}</span>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={form.streaming}
              disabled={saving}
              onChange={(event) => patch({ streaming: event.target.checked })}
            />
            <span>{t("providers.field_streaming_hint")}</span>
          </label>
        </div>
      </div>

      <Field
        label={t("providers.field_api_key")}
        htmlFor="provider-form-api-key"
        hint={
          editing
            ? `${t("providers.field_api_key_edit_hint")} ${t("providers.field_api_key_hint")}`
            : t("providers.field_api_key_hint")
        }
      >
        <input
          id="provider-form-api-key"
          className="input"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={form.api_key}
          disabled={saving}
          placeholder={t("providers.field_api_key_placeholder")}
          onChange={(event) => patch({ api_key: event.target.value })}
        />
      </Field>

      <Field
        label={t("providers.field_headers")}
        htmlFor="provider-form-headers"
        hint={t("providers.field_headers_hint")}
        error={errors.headers}
      >
        <textarea
          id="provider-form-headers"
          className="input mono"
          rows={4}
          spellCheck={false}
          value={form.headers}
          disabled={saving}
          placeholder={t("providers.field_headers_placeholder")}
          aria-describedby="provider-form-headers-issues"
          onChange={(event) => patch({ headers: event.target.value })}
        />
      </Field>

      {parsed.issues.length > 0 ? (
        <ul className="headers-issues" id="provider-form-headers-issues" role="alert">
          {parsed.issues.map((issue) => (
            <li key={`${issue.line}-${issue.code}`}>
              {t(`providers.header_error_${issue.code}`, { line: issue.line })}
            </li>
          ))}
        </ul>
      ) : null}

      {preview.length > 0 ? (
        <div className="headers-preview-block">
          <p className="field-label">
            {t("providers.headers_preview")} ({preview.length})
          </p>
          <ul className="headers-preview">
            {preview.map((entry) => (
              <li key={entry.name}>
                <code>{entry.name}</code>
                <span>{entry.value}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Modal>
  );
}
