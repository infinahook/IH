import { useEffect, useState } from "react";
import { Download, RefreshCw, ShieldCheck } from "lucide-react";
import { api, toAppError, type AppError, type UpdateStatus } from "../lib/api";
import { useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "./Badge";
import { ErrorBanner } from "./ErrorBanner";
import { LoadingRow, Spinner } from "./EmptyState";
import { Modal } from "./Modal";
import { Notice } from "./Notice";

export function UpdatesPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { t, navigate, clearUpdateAvailable } = useApp();
  const [version, setVersion] = useState<string | null>(null);
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [installApproval, setInstallApproval] = useState<string | null>(null);
  const [installPlan, setInstallPlan] = useState<string | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [installResult, setInstallResult] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const check = useAction();
  const download = useAction();
  const install = useAction();

  useEffect(() => {
    if (!open) {
      return undefined;
    }
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    setInstallResult(null);
    setInstallApproval(null);
    setInstallPlan(null);
    setConfirmed(false);
    Promise.allSettled([api.updates.status(), api.appVersion()]).then((results) => {
      if (cancelled) {
        return;
      }
      const [statusResult, versionResult] = results;
      if (versionResult.status === "fulfilled") {
        setVersion(versionResult.value);
      }
      if (statusResult.status === "fulfilled") {
        setStatus(statusResult.value);
      }
      const failure = results.find((result) => result.status === "rejected");
      if (failure && failure.status === "rejected") {
        setLoadError(toAppError(failure.reason));
      }
      setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const state = loading ? "loading" : status?.state ?? "idle";
  const availableVersion = status?.version ?? version ?? "—";

  const handleCheck = async () => {
    setInstallResult(null);
    setLoadError(null);
    await check.run(async () => {
      const next = await api.updates.check();
      setStatus(next);
      if (next.state === "up_to_date" || next.state === "not_configured") {
        clearUpdateAvailable();
      }
    });
  };

  const handleDownload = async () => {
    await download.run(async () => {
      const next = await api.updates.download();
      setStatus(next);
    });
  };

  const handleRequestInstall = async () => {
    const prepared = await install.run(async () => {
      const plan = await api.updates.plan();
      const approvalId = await api.updates.requestApproval();
      return { plan, approvalId };
    });
    if (prepared) {
      setInstallPlan(prepared.plan);
      setInstallApproval(prepared.approvalId);
      setConfirmed(false);
      setInstallResult(null);
    }
  };

  const handleApply = async () => {
    if (!installApproval) {
      return;
    }
    const result = await install.run(() => api.updates.apply(installApproval));
    if (typeof result === "string") {
      setInstallResult(result);
      setInstallApproval(null);
      clearUpdateAvailable();
      setStatus((current) => (current ? { ...current, state: "installed" } : current));
    }
  };

  const closePanel = () => {
    setInstallApproval(null);
    setInstallPlan(null);
    setConfirmed(false);
    onClose();
  };

  const goToSettings = () => {
    closePanel();
    navigate("settings");
  };

  return (
    <>
      <Modal
        open={open}
        title={t("updates.title")}
        onClose={closePanel}
        size="wide"
        footer={
          <>
            <button type="button" className="ghost" onClick={closePanel}>
              {t("common.close")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void handleCheck()}
              disabled={check.busy || download.busy || install.busy}
              aria-busy={check.busy}
            >
              {check.busy ? <Spinner /> : <RefreshCw size={14} aria-hidden="true" />}
              {t("common.check_updates")}
            </button>
          </>
        }
      >
        <div className="updates-panel">
          <div className="updates-meta">
            <Badge tone="neutral">{t("updates.current_version", { version: version ?? "…" })}</Badge>
            {state === "verified" || state === "installed" ? (
              <Badge tone="success">{t("updates.verified_badge")}</Badge>
            ) : null}
          </div>

          {loading ? <LoadingRow label={t("common.loading")} /> : null}
          {loadError ? <ErrorBanner error={loadError} onRetry={handleCheck} /> : null}

          {state === "not_configured" ? (
            <Notice tone="warning" title={t("updates.not_configured")}>
              <p>{t("updates.not_configured_hint")}</p>
              {status?.error ? <p className="mono">{status.error}</p> : null}
              <button type="button" className="ghost small" onClick={goToSettings}>
                {t("updates.open_settings")}
              </button>
            </Notice>
          ) : null}

          {state === "up_to_date" ? (
            <Notice tone="success" title={t("updates.up_to_date")}>
              <p>{t("updates.current_version", { version: version ?? "—" })}</p>
            </Notice>
          ) : null}

          {state === "available" ? (
            <Notice tone="info" title={t("updates.available", { version: availableVersion })}>
              <p className="prewrap">{status?.notes ?? t("updates.no_notes")}</p>
              <button
                type="button"
                className="primary"
                onClick={() => void handleDownload()}
                disabled={download.busy}
                aria-busy={download.busy}
              >
                {download.busy ? <Spinner /> : <Download size={14} aria-hidden="true" />}
                {download.busy ? t("updates.downloading") : t("updates.download")}
              </button>
            </Notice>
          ) : null}

          {state === "error" ? (
            <Notice tone="error" title={t("updates.error_state")}>
              <p>{t("updates.error_hint")}</p>
              {status?.error ? <p className="mono">{status.error}</p> : null}
            </Notice>
          ) : null}

          {state === "verified" || state === "installed" ? (
            <Notice tone="success" title={t("updates.verified")}>
              <p>{t("updates.download_hint")}</p>
              <ul className="check-list">
                <li>{t("updates.signature_ok")}</li>
                <li>{t("updates.digest_ok")}</li>
              </ul>
              <button
                type="button"
                className="primary"
                onClick={() => void handleRequestInstall()}
                disabled={install.busy || state === "installed"}
              >
                {install.busy ? <Spinner /> : <ShieldCheck size={14} aria-hidden="true" />}
                {t("updates.install_now")}
              </button>
            </Notice>
          ) : null}

          {state === "idle" ? <p className="muted">{t("updates.state_idle")}</p> : null}
          {!["loading", "idle", "not_configured", "up_to_date", "available", "error", "verified", "installed"].includes(
            state,
          ) ? (
            <Notice tone="info" title={state}>
              {status?.error ? <p className="mono">{status.error}</p> : null}
            </Notice>
          ) : null}

          {installResult ? (
            <Notice tone="success" title={t("updates.install_result")}>
              <p className="prewrap">{installResult}</p>
            </Notice>
          ) : null}

          {check.error ? <ErrorBanner error={check.error} /> : null}
          {download.error ? <ErrorBanner error={download.error} /> : null}
          {install.error && !installApproval ? <ErrorBanner error={install.error} /> : null}
          {state === "verified" || state === "installed" ? (
            <p className="muted small">{t("updates.signature_hint")}</p>
          ) : null}
        </div>
      </Modal>

      <Modal
        open={installApproval !== null}
        title={t("updates.install_confirm_title", { version: availableVersion })}
        onClose={() => setInstallApproval(null)}
        footer={
          <>
            <button
              type="button"
              className="ghost"
              onClick={() => setInstallApproval(null)}
              disabled={install.busy}
            >
              {t("updates.cancel_install")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void handleApply()}
              disabled={!confirmed || install.busy}
              aria-busy={install.busy}
            >
              {install.busy ? <Spinner /> : <ShieldCheck size={14} aria-hidden="true" />}
              {install.busy ? t("updates.installing") : t("updates.confirm_install")}
            </button>
          </>
        }
      >
        <Notice tone="success" title={t("updates.preflight")}>
          <ul className="check-list">
            <li>{t("updates.signature_ok")}</li>
            <li>{t("updates.digest_ok")}</li>
          </ul>
        </Notice>
        <p>{t("updates.install_confirm_text")}</p>
        {installPlan ? (
          <>
            <h3 className="section-title">{t("updates.install_plan")}</h3>
            <pre className="code-block">{installPlan}</pre>
          </>
        ) : null}
        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={confirmed}
            onChange={(event) => setConfirmed(event.target.checked)}
          />
          <span>{t("updates.confirm_checkbox")}</span>
        </label>
        <p className="muted small">{t("updates.confirmation_required")}</p>
        {install.error ? <ErrorBanner error={install.error} /> : null}
      </Modal>
    </>
  );
}

export function UpdatesButton({ onClick }: { onClick: () => void }) {
  const { t } = useApp();
  return (
    <button type="button" className="ghost" onClick={onClick}>
      <RefreshCw size={15} aria-hidden="true" />
      {t("updates.title")}
    </button>
  );
}
