import { useEffect, useState } from "react";
import { Save, Settings2, Undo2 } from "lucide-react";
import type { AppConfig, LogLevel } from "../lib/api";
import { api } from "../lib/api";
import type { Locale } from "../lib/i18n";
import { formatBytes } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { UpdatesPanel } from "../components/UpdatesPanel";

const LOG_LEVELS: LogLevel[] = ["trace", "debug", "info", "warn", "error"];
const MAX_OUTPUT_MIN = 64 * 1024;
const MAX_OUTPUT_MAX = 64 * 1024 * 1024;

export function SettingsPage() {
  const {
    t,
    config,
    bootError,
    reloadCore,
    refreshConfig,
    saveConfig,
    setLocale,
    locale,
  } = useApp();
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [updatesOpen, setUpdatesOpen] = useState(false);
  const version = useAsync(() => api.appVersion(), []);

  useEffect(() => {
    if (config && draft === null) {
      setDraft(config);
    }
  }, [config, draft]);

  if (!config || !draft) {
    return (
      <>
        <PageHeader title={t("settings.title")} />
        {bootError ? <ErrorBanner error={bootError} onRetry={() => void reloadCore()} /> : <LoadingRow label={t("app.loading")} />}
      </>
    );
  }

  const comparableDraft: AppConfig = {
    ...draft,
    locale: config.locale,
    last_project: config.last_project,
    recent_projects: config.recent_projects,
  };
  const dirty = JSON.stringify(comparableDraft) !== JSON.stringify(config);

  const patch = (updater: (current: AppConfig) => AppConfig) => {
    setDraft((current) => (current ? updater(current) : current));
    setErrors({});
    setMessage(null);
  };

  const validate = (next: AppConfig): Record<string, string> => {
    const found: Record<string, string> = {};
    if (
      !Number.isInteger(next.resources.max_concurrent_agents) ||
      next.resources.max_concurrent_agents < 1 ||
      next.resources.max_concurrent_agents > 4
    ) {
      found.maxAgents = t("settings.validation_range", { min: 1, max: 4 });
    }
    if (
      !Number.isInteger(next.resources.max_output_bytes_per_session) ||
      next.resources.max_output_bytes_per_session < MAX_OUTPUT_MIN ||
      next.resources.max_output_bytes_per_session > MAX_OUTPUT_MAX
    ) {
      found.outputLimit = t("settings.output_limit_hint");
    }
    if (
      !Number.isInteger(next.updates.check_interval_minutes) ||
      next.updates.check_interval_minutes < 15
    ) {
      found.checkInterval = t("settings.check_interval_hint");
    }
    const endpoint = next.updates.endpoint?.trim() ?? "";
    if (endpoint.length > 0 && !endpoint.startsWith("https://")) {
      found.endpoint = t("settings.https_required");
    }
    const key = next.updates.public_key_hex?.trim() ?? "";
    if (key.length > 0 && !/^[0-9a-fA-F]{64}$/.test(key)) {
      found.publicKey = t("settings.key_hex_invalid");
    }
    if (!["trace", "debug", "info", "warn", "error"].includes(next.logging.level)) {
      found.logging = t("settings.logging_level");
    }
    return found;
  };

  const handleSave = async () => {
    setSaving(true);
    setMessage(null);
    const base = await refreshConfig();
    if (!base) {
      setSaving(false);
      return;
    }
    // Seuls les champs éditables dans Paramètres sont appliqués ; le reste de
    // la configuration fraîche (projets, fournisseurs…) est préservé.
    const normalized: AppConfig = {
      ...base,
      theme: draft.theme,
      resources: draft.resources,
      logging: draft.logging,
      updates: {
        ...draft.updates,
        endpoint: draft.updates.endpoint?.trim() || null,
        public_key_hex: draft.updates.public_key_hex?.trim() || null,
      },
    };
    const found = validate(normalized);
    setErrors(found);
    if (Object.keys(found).length > 0) {
      setSaving(false);
      return;
    }
    const saved = await saveConfig(normalized);
    setSaving(false);
    if (saved) {
      setDraft(saved);
      setMessage(t("settings.saved_msg"));
    }
  };

  const resetDraft = () => {
    setDraft(config);
    setErrors({});
    setMessage(null);
  };

  const changeLocale = async (next: Locale) => {
    setDraft((current) => (current ? { ...current, locale: next } : current));
    await setLocale(next);
  };

  return (
    <>
      <PageHeader
        title={t("settings.title")}
        subtitle={t("settings.save_hint")}
        actions={
          <>
            {dirty ? <Badge tone="warning">{t("settings.unsaved")}</Badge> : null}
            <button type="button" className="ghost" onClick={resetDraft} disabled={!dirty}>
              <Undo2 size={15} aria-hidden="true" />
              {t("common.reset")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void handleSave()}
              disabled={saving}
              aria-busy={saving}
            >
              {saving ? <Spinner /> : <Save size={15} aria-hidden="true" />}
              {saving ? t("common.saving") : t("common.save")}
            </button>
          </>
        }
      />

      {message ? <Notice tone="success">{message}</Notice> : null}

      <div className="grid grid-2">
        <section className="card" aria-labelledby="settings-general">
          <div className="section-heading">
            <h2 id="settings-general">{t("settings.general")}</h2>
            <Settings2 size={17} aria-hidden="true" className="muted-icon" />
          </div>
          <Field
            label={t("settings.language")}
            htmlFor="settings-locale"
            hint={t("settings.language_hint")}
          >
            <select
              id="settings-locale"
              className="input"
              value={locale}
              onChange={(event) => void changeLocale(event.target.value === "en" ? "en" : "fr")}
            >
              <option value="fr">{t("common.french")}</option>
              <option value="en">{t("common.english")}</option>
            </select>
          </Field>
          <Field label={t("settings.logging_level")} htmlFor="settings-logging" error={errors.logging}>
            <select
              id="settings-logging"
              className="input"
              value={draft.logging.level}
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  logging: { ...current.logging, level: event.target.value },
                }))
              }
            >
              {LOG_LEVELS.map((level) => (
                <option key={level} value={level}>
                  {t(`settings.level_${level}`)}
                </option>
              ))}
            </select>
          </Field>
          <p className="muted small">{t("settings.version_info", { version: version.data ?? "…" })}</p>
        </section>

        <section className="card" aria-labelledby="settings-appearance">
          <div className="section-heading">
            <h2 id="settings-appearance">{t("settings.appearance")}</h2>
          </div>
          <Field label={t("settings.theme")} htmlFor="settings-theme">
            <select
              id="settings-theme"
              className="input"
              value={draft.theme}
              onChange={(event) =>
                patch((current) => ({ ...current, theme: event.target.value }))
              }
            >
              <option value="dark">{t("settings.dark")}</option>
              <option value="light">{t("settings.light")}</option>
              <option value="system">{t("settings.system")}</option>
            </select>
          </Field>
          <p className="muted small">
            {t("settings.theme")} :{" "}
            {draft.theme === "light"
              ? t("settings.light")
              : draft.theme === "system"
                ? t("settings.system")
                : t("settings.dark")}
          </p>
          <Field
            label={t("settings.update_channel")}
            htmlFor="settings-channel"
            hint={t("settings.update_hint")}
          >
            <input
              id="settings-channel"
              className="input"
              list="settings-channel-list"
              value={draft.updates.channel}
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  updates: { ...current.updates, channel: event.target.value },
                }))
              }
            />
            <datalist id="settings-channel-list">
              <option value="stable">{t("settings.channel_stable")}</option>
              <option value="beta">{t("settings.channel_beta")}</option>
            </datalist>
          </Field>
        </section>
      </div>

      <section className="card" aria-labelledby="settings-resources">
        <div className="section-heading">
          <h2 id="settings-resources">{t("settings.resources")}</h2>
        </div>
        <p className="muted small">{t("settings.resources_hint")}</p>
        <div className="form-grid">
          <Field
            label={t("settings.max_agents")}
            htmlFor="settings-max-agents"
            hint={t("settings.max_agents_hint")}
            error={errors.maxAgents}
          >
            <input
              id="settings-max-agents"
              className="input"
              type="number"
              min={1}
              max={4}
              value={draft.resources.max_concurrent_agents}
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  resources: {
                    ...current.resources,
                    max_concurrent_agents: Number(event.target.value),
                  },
                }))
              }
            />
          </Field>
          <Field
            label={t("settings.output_limit")}
            htmlFor="settings-output-limit"
            hint={`${t("settings.output_limit_hint")} · ${formatBytes(
              draft.resources.max_output_bytes_per_session,
            )}`}
            error={errors.outputLimit}
          >
            <input
              id="settings-output-limit"
              className="input"
              type="number"
              step={1024}
              min={MAX_OUTPUT_MIN}
              max={MAX_OUTPUT_MAX}
              value={draft.resources.max_output_bytes_per_session}
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  resources: {
                    ...current.resources,
                    max_output_bytes_per_session: Number(event.target.value),
                  },
                }))
              }
            />
          </Field>
        </div>
      </section>

      <section className="card" aria-labelledby="settings-updates">
        <div className="section-heading">
          <h2 id="settings-updates">{t("settings.updates_section")}</h2>
          <button type="button" className="ghost small" onClick={() => setUpdatesOpen(true)}>
            {t("settings.manage_updates")}
          </button>
        </div>
        <p className="muted small">{t("settings.update_hint")}</p>
        <div className="form-grid">
          <Field
            label={t("settings.update_endpoint")}
            htmlFor="settings-endpoint"
            hint={t("settings.update_endpoint_hint")}
            error={errors.endpoint}
          >
            <input
              id="settings-endpoint"
              className="input mono"
              value={draft.updates.endpoint ?? ""}
              placeholder="https://…/manifest.json"
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  updates: { ...current.updates, endpoint: event.target.value },
                }))
              }
            />
          </Field>
          <Field
            label={t("settings.public_key")}
            htmlFor="settings-public-key"
            hint={t("settings.public_key_hint")}
            error={errors.publicKey}
          >
            <input
              id="settings-public-key"
              className="input mono"
              value={draft.updates.public_key_hex ?? ""}
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  updates: { ...current.updates, public_key_hex: event.target.value },
                }))
              }
            />
          </Field>
          <Field
            label={t("settings.check_interval")}
            htmlFor="settings-interval"
            hint={t("settings.check_interval_hint")}
            error={errors.checkInterval}
          >
            <input
              id="settings-interval"
              className="input"
              type="number"
              min={15}
              value={draft.updates.check_interval_minutes}
              onChange={(event) =>
                patch((current) => ({
                  ...current,
                  updates: {
                    ...current.updates,
                    check_interval_minutes: Number(event.target.value),
                  },
                }))
              }
            />
          </Field>
          <div className="field">
            <span className="field-label">{t("settings.advanced_updates")}</span>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={draft.updates.auto_download}
                onChange={(event) =>
                  patch((current) => ({
                    ...current,
                    updates: { ...current.updates, auto_download: event.target.checked },
                  }))
                }
              />
              <span>{t("settings.auto_download")}</span>
            </label>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={draft.updates.allow_prerelease}
                onChange={(event) =>
                  patch((current) => ({
                    ...current,
                    updates: { ...current.updates, allow_prerelease: event.target.checked },
                  }))
                }
              />
              <span>{t("settings.allow_prerelease")}</span>
            </label>
            <p className="field-hint">{t("settings.endpoint_empty_hint")}</p>
          </div>
        </div>
      </section>

      {version.error ? <ErrorBanner error={version.error} /> : null}

      <UpdatesPanel open={updatesOpen} onClose={() => setUpdatesOpen(false)} />
    </>
  );
}
