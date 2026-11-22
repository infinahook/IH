import { AlertTriangle, RefreshCw, X } from "lucide-react";
import type { AppError } from "../lib/api";
import { describeError } from "../lib/errors";
import { useApp } from "../lib/store";

export function ErrorBanner({
  error,
  onRetry,
  onDismiss,
}: {
  error: AppError;
  onRetry?: () => void;
  onDismiss?: () => void;
}) {
  const { t } = useApp();
  const described = describeError(t, error);
  return (
    <div className="banner banner-error" role="alert">
      <AlertTriangle size={18} aria-hidden="true" />
      <div className="banner-content">
        <strong>{described.summary}</strong>
        {described.detail ? <p className="banner-detail">{described.detail}</p> : null}
        <p className="banner-meta">
          <span>{t("errors.code_label", { code: described.code })}</span>
          {described.fixable ? <span>{t("errors.fixable")}</span> : null}
        </p>
      </div>
      <div className="banner-actions">
        {onRetry ? (
          <button type="button" className="ghost" onClick={onRetry}>
            <RefreshCw size={14} aria-hidden="true" />
            {t("common.retry")}
          </button>
        ) : null}
        {onDismiss ? (
          <button
            type="button"
            className="icon-button"
            aria-label={t("common.close")}
            onClick={onDismiss}
          >
            <X size={15} />
          </button>
        ) : null}
      </div>
    </div>
  );
}
