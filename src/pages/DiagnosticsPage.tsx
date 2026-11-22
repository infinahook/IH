import { useState } from "react";
import { Download, FolderOpen, RefreshCw, Stethoscope } from "lucide-react";
import { api, type DiagnosticsReport, type PathsInfo, type PlatformInfo } from "../lib/api";
import { downloadText } from "../lib/download";
import { formatBytes, formatDate } from "../lib/format";
import { useTauriEvent } from "../lib/events";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { JsonView } from "../components/JsonView";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

export function DiagnosticsPage() {
  const { t, locale, health } = useApp();
  const platform = useAsync<PlatformInfo>(() => api.platformInfo(), []);
  const paths = useAsync<PathsInfo>(() => api.pathsInfo(), []);
  const report = useAsync<DiagnosticsReport>(() => api.diagnosticsReport(), []);
  const logs = useAsync<string[]>(() => api.logTail(200), []);
  const openLogs = useAction();
  const [message, setMessage] = useState<string | null>(null);
  const logsPath = paths.data?.logs ?? null;
  const logEventError = useTauriEvent<{ level: string; message: string }>("ih://log", (payload) => {
    logs.setData((current) => {
      const next = [...(current ?? []), `[${payload.level}] ${payload.message}`];
      return next.slice(-200);
    });
  });

  const familyLabel = (family: PlatformInfo["family"]): string => {
    switch (family) {
      case "arch":
        return t("diagnostics.family_arch");
      case "debian":
        return t("diagnostics.family_debian");
      default:
        return t("diagnostics.family_other");
    }
  };

  const exportReport = () => {
    if (!report.data) {
      return;
    }
    downloadText("ih-diagnostics.json", JSON.stringify(report.data, null, 2));
    setMessage(t("diagnostics.exported"));
  };

  const refreshAll = () => {
    platform.reload();
    paths.reload();
    report.reload();
    logs.reload();
    setMessage(null);
  };

  return (
    <>
      <PageHeader
        title={t("diagnostics.title")}
        subtitle={t("diagnostics.report_hint")}
        actions={
          <>
            <button type="button" className="ghost" onClick={refreshAll}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={exportReport}
              disabled={!report.data}
            >
              <Download size={15} aria-hidden="true" />
              {t("diagnostics.export_report")}
            </button>
          </>
        }
      />

      {message ? <Notice tone="success">{message}</Notice> : null}
      {platform.error ? <ErrorBanner error={platform.error} onRetry={platform.reload} /> : null}
      {paths.error ? <ErrorBanner error={paths.error} onRetry={paths.reload} /> : null}
      {report.error ? <ErrorBanner error={report.error} onRetry={report.reload} /> : null}
      {logs.error ? <ErrorBanner error={logs.error} onRetry={logs.reload} /> : null}
      {openLogs.error ? <ErrorBanner error={openLogs.error} /> : null}
      {logEventError ? <ErrorBanner error={logEventError} /> : null}

      <div className="grid grid-2">
        <section className="card" aria-labelledby="diagnostics-system">
          <div className="section-heading">
            <h2 id="diagnostics-system">{t("diagnostics.system")}</h2>
            <Badge tone={health?.core_online ? "success" : "danger"}>
              {health?.core_online ? t("status.online") : t("status.offline")}
            </Badge>
          </div>
          {platform.loading ? (
            <LoadingRow label={t("common.loading")} />
          ) : platform.data ? (
            <dl className="detail-grid">
              <div>
                <dt>{t("diagnostics.distro")}</dt>
                <dd>{platform.data.os_name}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.os_version")}</dt>
                <dd>{platform.data.os_version || "—"}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.family")}</dt>
                <dd>{familyLabel(platform.data.family)}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.arch")}</dt>
                <dd className="mono">{platform.data.arch}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.kernel")}</dt>
                <dd className="mono">{platform.data.kernel || "—"}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.cpu_cores")}</dt>
                <dd>{platform.data.cpu_cores}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.ram")}</dt>
                <dd>{formatBytes(platform.data.ram_mb * 1024 * 1024)}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.gpu")}</dt>
                <dd>{platform.data.gpu ?? t("common.unknown")}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.recommended_agents")}</dt>
                <dd>{platform.data.recommended_max_agents}</dd>
              </div>
            </dl>
          ) : null}
          {platform.data ? (
            <div className="shell-hint">
              <span className="small muted">{t("diagnostics.shell_hint")}</span>
              <pre className="code-block">{t(`diagnostics.hint_${platform.data.family}`)}</pre>
            </div>
          ) : null}
        </section>

        <section className="card" aria-labelledby="diagnostics-paths">
          <div className="section-heading">
            <h2 id="diagnostics-paths">{t("diagnostics.paths")}</h2>
            <button
              type="button"
              className="ghost small"
              onClick={() => {
                if (logsPath) {
                  void openLogs.run(() => api.openPath(logsPath));
                }
              }}
              disabled={!logsPath || openLogs.busy}
              aria-busy={openLogs.busy}
            >
              {openLogs.busy ? <Spinner /> : <FolderOpen size={13} aria-hidden="true" />}
              {t("diagnostics.open_logs")}
            </button>
          </div>
          <p className="muted small">{t("diagnostics.paths_hint")}</p>
          {paths.loading ? (
            <LoadingRow label={t("common.loading")} />
          ) : paths.data ? (
            <dl className="detail-grid single">
              <div>
                <dt>{t("diagnostics.data_root")}</dt>
                <dd className="mono">{paths.data.root}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.config_file")}</dt>
                <dd className="mono">{paths.data.config_file}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.database")}</dt>
                <dd className="mono">{paths.data.database}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.logs")}</dt>
                <dd className="mono">{paths.data.logs}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.skills_dir")}</dt>
                <dd className="mono">{paths.data.skills}</dd>
              </div>
              <div>
                <dt>{t("diagnostics.plugins_dir")}</dt>
                <dd className="mono">{paths.data.plugins}</dd>
              </div>
            </dl>
          ) : null}
        </section>
      </div>

      <section className="card" aria-labelledby="diagnostics-report">
        <div className="section-heading">
          <h2 id="diagnostics-report">{t("diagnostics.report")}</h2>
          <Stethoscope size={17} aria-hidden="true" className="muted-icon" />
        </div>
        {report.loading ? (
          <LoadingRow label={t("common.loading")} />
        ) : report.data ? (
          <>
            <p className="muted small">
              {t("diagnostics.generated_at")} : {formatDate(report.data.generated_at, locale)} ·{" "}
              {t("diagnostics.app_version")} : {report.data.app_version} ·{" "}
              {t("diagnostics.locale")} : {report.data.locale}
            </p>
            <div className="grid grid-3 report-counts">
              <div className="mini-stat">
                <span className="mini-stat-value">{report.data.projects}</span>
                <span className="small muted">{t("diagnostics.projects_count")}</span>
              </div>
              <div className="mini-stat">
                <span className="mini-stat-value">{report.data.threads}</span>
                <span className="small muted">{t("diagnostics.threads_count")}</span>
              </div>
              <div className="mini-stat">
                <span className="mini-stat-value">{report.data.tasks}</span>
                <span className="small muted">{t("diagnostics.tasks_count")}</span>
              </div>
              <div className="mini-stat">
                <span className="mini-stat-value">{report.data.memory_entries}</span>
                <span className="small muted">{t("diagnostics.memory_count")}</span>
              </div>
              <div className="mini-stat">
                <span className="mini-stat-value">{report.data.installed_skills}</span>
                <span className="small muted">{t("diagnostics.skills_count")}</span>
              </div>
              <div className="mini-stat">
                <span className="mini-stat-value">{report.data.installed_plugins}</span>
                <span className="small muted">{t("diagnostics.plugins_count")}</span>
              </div>
            </div>
            <p className="mono small">
              {t("diagnostics.database")} : {report.data.database}
            </p>
            <details className="details">
              <summary>{t("diagnostics.report")}</summary>
              <JsonView value={report.data} maxHeight={360} />
            </details>
            <p className="muted small">{t("diagnostics.redacted")}</p>
          </>
        ) : null}
      </section>

      <section className="card" aria-labelledby="diagnostics-logs">
        <div className="section-heading">
          <h2 id="diagnostics-logs">{t("diagnostics.logs")}</h2>
          <button type="button" className="ghost small" onClick={() => logs.reload()}>
            <RefreshCw size={13} aria-hidden="true" />
            {t("diagnostics.refresh_logs")}
          </button>
        </div>
        <p className="muted small">{t("diagnostics.log_hint")}</p>
        {logs.loading ? (
          <LoadingRow label={t("common.loading")} />
        ) : logs.data && logs.data.length > 0 ? (
          <>
            <p className="small muted">
              {t("diagnostics.tail_lines", { count: logs.data.length })}
            </p>
            <pre className="code-block log-view">{logs.data.join("\n")}</pre>
          </>
        ) : (
          <p className="muted">{t("diagnostics.no_logs")}</p>
        )}
      </section>
    </>
  );
}
