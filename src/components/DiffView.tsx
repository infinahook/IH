import { Check, FileDiff, RefreshCw, X } from "lucide-react";
import type { AppError, DiffPreview } from "../lib/api";
import {
  diffStatsLabel,
  hunkLabel,
  summarizeDiff,
  type DiffLine,
  type DiffHunk,
} from "../lib/diff";
import { useApp } from "../lib/store";
import { Badge } from "./Badge";
import { Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";

function lineClass(line: DiffLine): string {
  switch (line.kind) {
    case "add":
      return "diff-row diff-row-add";
    case "remove":
      return "diff-row diff-row-remove";
    default:
      return "diff-row diff-row-context";
  }
}

function sign(line: DiffLine): string {
  if (line.kind === "add") {
    return "+";
  }
  if (line.kind === "remove") {
    return "\u2212";
  }
  return " ";
}

/**
 * Vue d'un diff préparé par le noyau : hunks avec contexte, ajouts et
 * suppressions, numéros de ligne des deux versions, compteurs et mention de
 * troncature. Aucune ligne n'est recalculée côté interface.
 */
export function DiffView({
  preview,
  busy,
  error,
  onAccept,
  onReject,
  onRetryPrepare,
  onClose,
  errorIsConflict,
}: {
  preview: DiffPreview;
  busy: boolean;
  error: AppError | null;
  onAccept: () => void;
  onReject: () => void;
  onRetryPrepare: () => void;
  onClose: () => void;
  errorIsConflict: boolean;
}) {
  const { t } = useApp();
  const summary = summarizeDiff(preview.diff);
  const stats = diffStatsLabel(preview.additions, preview.deletions);

  const renderHunk = (hunk: DiffHunk, index: number) => (
    <div className="diff-hunk-block" key={`${hunk.old_start}-${hunk.new_start}-${index}`}>
      <div className="diff-hunk-header">
        <span className="mono">{hunkLabel(hunk)}</span>
        <span className="small muted">
          {t("workspace.diff_hunk_counts", {
            additions: hunk.lines.filter((line) => line.kind === "add").length,
            deletions: hunk.lines.filter((line) => line.kind === "remove").length,
          })}
        </span>
      </div>
      <div className="diff-hunk-lines" role="table" aria-label={hunkLabel(hunk)}>
        {hunk.lines.map((line, lineIndex) => (
          <div
            className={lineClass(line)}
            role="row"
            key={`${line.old_line ?? "n"}-${line.new_line ?? "n"}-${lineIndex}`}
          >
            <span className="diff-line-number" role="cell" aria-hidden="true">
              {line.old_line ?? ""}
            </span>
            <span className="diff-line-number" role="cell" aria-hidden="true">
              {line.new_line ?? ""}
            </span>
            <span className="diff-sign" role="cell" aria-hidden="true">
              {sign(line)}
            </span>
            <span className="diff-text mono" role="cell">
              {line.text.length > 0 ? line.text : "\u00a0"}
            </span>
          </div>
        ))}
      </div>
    </div>
  );

  return (
    <section className="diff-panel" aria-labelledby="workspace-diff-title">
      <header className="diff-panel-head">
        <div className="diff-panel-heading">
          <FileDiff size={16} aria-hidden="true" />
          <h2 id="workspace-diff-title">{t("workspace.diff_title")}</h2>
          <span className="mono small">{preview.path}</span>
        </div>
        <div className="chat-badges">
          <Badge tone="success">+{preview.additions}</Badge>
          <Badge tone="danger">
            {"\u2212"}
            {preview.deletions}
          </Badge>
          <span className="mono small" aria-label={t("workspace.diff_stats")}>
            {stats}
          </span>
          {summary.truncated ? (
            <Badge tone="warning">{t("workspace.diff_truncated")}</Badge>
          ) : null}
          <span className="small muted">
            {t("workspace.diff_line_counts", {
              old: summary.oldLines,
              new: summary.newLines,
            })}
          </span>
        </div>
        <div className="button-row compact">
          <button
            type="button"
            className="primary"
            onClick={onAccept}
            disabled={busy}
            aria-busy={busy}
          >
            {busy ? <Spinner /> : <Check size={14} aria-hidden="true" />}
            {t("workspace.diff_accept")}
          </button>
          <button type="button" className="ghost" onClick={onReject} disabled={busy}>
            <X size={14} aria-hidden="true" />
            {t("workspace.diff_reject")}
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label={t("workspace.diff_close")}
            title={t("workspace.diff_close")}
            onClick={onClose}
            disabled={busy}
          >
            <X size={15} />
          </button>
        </div>
      </header>

      {summary.truncated ? (
        <p className="notice notice-warning" role="status">
          {t("workspace.diff_truncated_hint")}
        </p>
      ) : null}

      {error ? (
        errorIsConflict ? (
          <div className="banner banner-error" role="alert">
            <div className="banner-content">
              <strong>{t("workspace.diff_conflict_title")}</strong>
              <p className="banner-detail">{t("workspace.diff_conflict_text")}</p>
              <p className="banner-meta">
                <span>{t("errors.code_label", { code: error.code })}</span>
              </p>
            </div>
            <div className="banner-actions">
              <button type="button" className="ghost" onClick={onRetryPrepare} disabled={busy}>
                <RefreshCw size={14} aria-hidden="true" />
                {t("workspace.diff_reprepare")}
              </button>
            </div>
          </div>
        ) : (
          <ErrorBanner error={error} onRetry={onRetryPrepare} />
        )
      ) : null}

      <div className="diff-scroll">
        {summary.hunks === 0 ? (
          <p className="muted">{t("workspace.diff_empty")}</p>
        ) : (
          preview.diff.hunks.map(renderHunk)
        )}
      </div>
      <p className="small muted">{t("workspace.diff_footer")}</p>
    </section>
  );
}
