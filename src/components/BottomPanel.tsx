import { useEffect } from "react";
import {
  Check,
  ChevronDown,
  FileDiff,
  GitBranch,
  RefreshCw,
  Stethoscope,
  X,
} from "lucide-react";
import { api, type GitStatus, type PendingDiff } from "../lib/api";
import { formatDate } from "../lib/format";
import { gitStatusRows, stagedRows, unstagedRows } from "../lib/git";
import { useAsync } from "../lib/hooks";
import { useApp } from "../lib/store";
import type { BottomTab } from "../lib/layout";
import type { WorkspaceController } from "../lib/useWorkspace";
import { Badge } from "./Badge";
import { LoadingRow, Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";
import { Tabs, tabButtonId, tabPanelId } from "./Tabs";
import { TerminalPane } from "./TerminalPane";

const PANEL_PREFIX = "workspace-bottom";

function GitPanel({ workspace, projectId }: { workspace: WorkspaceController; projectId: string }) {
  const { t, locale } = useApp();
  const status = useAsync<GitStatus>(() => api.git.status(projectId), [projectId]);
  const rows = status.data ? gitStatusRows(status.data) : [];
  const staged = stagedRows(rows);
  const unstaged = unstagedRows(rows);
  const pending = workspace.pendingDiffs.filter((entry) => entry.project_id === projectId);

  useEffect(() => {
    void workspace.refreshPendingDiffs();
    // Chargement réel à l'ouverture de l'onglet Git.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  const pendingRow = (entry: PendingDiff) => (
    <li key={entry.id} className="pending-diff-row">
      <FileDiff size={13} aria-hidden="true" />
      <span className="mono small pending-diff-path" title={entry.path}>
        {entry.path}
      </span>
      <span className="small muted">{formatDate(entry.created_at, locale)}</span>
      <div className="button-row compact">
        <button
          type="button"
          className="ghost small"
          onClick={() => void workspace.acceptPendingDiff(entry.id)}
          disabled={workspace.pendingDiffBusy}
        >
          <Check size={12} aria-hidden="true" />
          {t("workspace.diff_accept")}
        </button>
        <button
          type="button"
          className="danger small"
          onClick={() => void workspace.rejectPendingDiff(entry.id)}
          disabled={workspace.pendingDiffBusy}
        >
          <X size={12} aria-hidden="true" />
          {t("workspace.diff_reject")}
        </button>
      </div>
    </li>
  );

  return (
    <div className="workspace-git">
      <div className="workspace-panel-toolbar">
        {status.loading ? <Spinner /> : null}
        {status.data ? (
          <>
            <span className="inline-status">
              <GitBranch size={13} aria-hidden="true" />
              <span className="mono small">{status.data.branch || t("git.current_branch")}</span>
            </span>
            {status.data.clean ? (
              <Badge tone="success">{t("git.clean")}</Badge>
            ) : (
              <>
                <Badge tone="accent">
                  {t("workspace.git_staged_count", { count: staged.length })}
                </Badge>
                <Badge tone="warning">
                  {t("workspace.git_unstaged_count", { count: unstaged.length })}
                </Badge>
              </>
            )}
            {status.data.ahead > 0 || status.data.behind > 0 ? (
              <span className="small muted">
                {t("git.ahead_behind", {
                  ahead: status.data.ahead,
                  behind: status.data.behind,
                })}
              </span>
            ) : null}
          </>
        ) : null}
        <button
          type="button"
          className="ghost small"
          onClick={() => {
            status.reload();
            void workspace.refreshPendingDiffs();
          }}
          disabled={status.loading}
        >
          {status.loading ? <Spinner /> : <RefreshCw size={12} aria-hidden="true" />}
          {t("common.refresh")}
        </button>
      </div>

      {status.error ? <ErrorBanner error={status.error} onRetry={status.reload} /> : null}
      {workspace.pendingDiffError ? (
        <ErrorBanner
          error={workspace.pendingDiffError}
          onRetry={() => void workspace.refreshPendingDiffs()}
        />
      ) : null}

      <div className="workspace-git-columns">
        <section aria-labelledby="workspace-git-staged">
          <h3 id="workspace-git-staged" className="section-title">
            {t("git.staged")} <span className="tab-count">{staged.length}</span>
          </h3>
          {staged.length === 0 ? (
            <p className="small muted">{t("git.no_changes")}</p>
          ) : (
            <ul className="workspace-git-list">
              {staged.map((row) => (
                <li key={`staged-${row.path}`}>
                  <span className="mono small">{row.path}</span>
                  <Badge tone="success">{row.stagedCode || "?"}</Badge>
                </li>
              ))}
            </ul>
          )}
        </section>
        <section aria-labelledby="workspace-git-unstaged">
          <h3 id="workspace-git-unstaged" className="section-title">
            {t("git.unstaged")} <span className="tab-count">{unstaged.length}</span>
          </h3>
          {unstaged.length === 0 ? (
            <p className="small muted">{t("git.no_changes")}</p>
          ) : (
            <ul className="workspace-git-list">
              {unstaged.map((row) => (
                <li key={`unstaged-${row.path}`}>
                  <span className="mono small">{row.path}</span>
                  <Badge tone="warning">{row.worktreeCode || "?"}</Badge>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      <section className="workspace-pending-diffs" aria-labelledby="workspace-pending-title">
        <h3 id="workspace-pending-title" className="section-title">
          {t("workspace.pending_diffs")} <span className="tab-count">{pending.length}</span>
        </h3>
        {pending.length === 0 ? (
          <p className="small muted">{t("workspace.no_pending_diffs")}</p>
        ) : (
          <ul className="pending-diff-list">{pending.map(pendingRow)}</ul>
        )}
      </section>
    </div>
  );
}

function ProblemsPanel() {
  const { t, locale } = useApp();
  const diagnostics = useAsync(() => api.diagnosticsReport(), []);
  const logs = useAsync(() => api.logTail(80), []);

  const report = diagnostics.data;
  const counters: Array<[string, number]> = report
    ? [
        [t("diagnostics.projects_count"), report.projects],
        [t("diagnostics.threads_count"), report.threads],
        [t("diagnostics.tasks_count"), report.tasks],
        [t("diagnostics.memory_count"), report.memory_entries],
        [t("diagnostics.skills_count"), report.installed_skills],
        [t("diagnostics.plugins_count"), report.installed_plugins],
      ]
    : [];

  return (
    <div className="workspace-problems">
      <div className="workspace-panel-toolbar">
        <Stethoscope size={13} aria-hidden="true" />
        <span className="small muted">
          {report
            ? t("workspace.problems_generated", {
                date: formatDate(report.generated_at, locale),
              })
            : t("common.loading")}
        </span>
        <button
          type="button"
          className="ghost small"
          onClick={() => {
            diagnostics.reload();
            logs.reload();
          }}
          disabled={diagnostics.loading || logs.loading}
        >
          {diagnostics.loading || logs.loading ? (
            <Spinner />
          ) : (
            <RefreshCw size={12} aria-hidden="true" />
          )}
          {t("common.refresh")}
        </button>
      </div>

      {diagnostics.error ? (
        <ErrorBanner error={diagnostics.error} onRetry={diagnostics.reload} />
      ) : null}
      {logs.error ? <ErrorBanner error={logs.error} onRetry={logs.reload} /> : null}

      {report ? (
        <div className="workspace-problems-summary">
          <p className="small">
            <strong>{t("diagnostics.system")}</strong>{" "}
            <span className="mono">
              {report.platform.os_name} · {report.platform.arch} · {report.platform.kernel}
            </span>
          </p>
          <p className="small muted">
            {t("diagnostics.app_version")} {report.app_version} · {t("diagnostics.locale")}{" "}
            {report.locale}
          </p>
          <ul className="workspace-counters">
            {counters.map(([label, value]) => (
              <li key={label}>
                <span className="small muted">{label}</span>
                <span className="mono">{value}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : diagnostics.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : null}

      <div className="workspace-logs">
        <h3 className="section-title">{t("diagnostics.logs")}</h3>
        {logs.loading && logs.data === null ? (
          <LoadingRow label={t("common.loading")} />
        ) : (logs.data ?? []).length === 0 ? (
          <p className="small muted">{t("diagnostics.no_logs")}</p>
        ) : (
          <pre className="workspace-log-tail mono" aria-label={t("diagnostics.logs")}>
            {(logs.data ?? []).join("\n")}
          </pre>
        )}
      </div>
    </div>
  );
}

/**
 * Panneau inférieur escamotable : Terminal (session conservée tant que le
 * panneau est ouvert), Git (statut + diffs en attente) et Problèmes
 * (diagnostic + journal). Les panneaux sont annoncés par les rôles ARIA.
 */
export function BottomPanel({
  workspace,
  projectId,
  activeTab,
  open,
  onTabChange,
  onClose,
}: {
  workspace: WorkspaceController;
  projectId: string;
  activeTab: BottomTab;
  /** Le panneau reste monté quand il est fermé : la session PTY survit. */
  open: boolean;
  onTabChange: (tab: BottomTab) => void;
  onClose: () => void;
}) {
  const { t } = useApp();
  const tabs = [
    { id: "terminal" as const, label: t("workspace.tab_terminal") },
    {
      id: "git" as const,
      label: t("workspace.tab_git"),
      detail: workspace.pendingDiffs.length > 0 ? String(workspace.pendingDiffs.length) : undefined,
    },
    { id: "problems" as const, label: t("workspace.tab_problems") },
  ];

  const panel = (id: BottomTab) => ({
    role: "tabpanel" as const,
    id: tabPanelId(PANEL_PREFIX, id),
    "aria-labelledby": tabButtonId(PANEL_PREFIX, id),
  });

  return (
    <section
      className="card workspace-bottom"
      id="workspace-bottom-panel"
      aria-label={t("workspace.bottom_panel")}
      hidden={!open}
    >
      <div className="workspace-bottom-head">
        <Tabs
          idPrefix={PANEL_PREFIX}
          label={t("workspace.bottom_panel")}
          tabs={tabs}
          active={activeTab}
          onChange={onTabChange}
        />
        <button
          type="button"
          className="icon-button"
          aria-label={t("workspace.hide_bottom")}
          title={t("workspace.hide_bottom")}
          onClick={onClose}
        >
          <ChevronDown size={15} />
        </button>
      </div>

      <div className="workspace-bottom-body">
        <div {...panel("terminal")} hidden={activeTab !== "terminal"} className="workspace-tab-panel">
          <TerminalPane key={projectId} projectId={projectId} />
        </div>
        <div {...panel("git")} hidden={activeTab !== "git"} className="workspace-tab-panel">
          {open && activeTab === "git" ? (
            <GitPanel workspace={workspace} projectId={projectId} />
          ) : null}
        </div>
        <div {...panel("problems")} hidden={activeTab !== "problems"} className="workspace-tab-panel">
          {open && activeTab === "problems" ? <ProblemsPanel /> : null}
        </div>
      </div>
    </section>
  );
}
