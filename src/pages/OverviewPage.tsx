import { useState } from "react";
import {
  ArrowRight,
  Bot,
  Cpu,
  FolderOpen,
  HardDrive,
  ListChecks,
  Monitor,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";
import { api } from "../lib/api";
import { formatBytes } from "../lib/format";
import { useAsync } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ErrorBanner } from "../components/ErrorBanner";
import { LoadingRow } from "../components/EmptyState";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { UpdatesPanel } from "../components/UpdatesPanel";

interface OverviewCounts {
  openTasks: number;
  activeSessions: number;
}

export function OverviewPage() {
  const {
    t,
    locale,
    health,
    platform,
    config,
    project,
    projects,
    selectProject,
    navigate,
    updateAvailable,
  } = useApp();
  const [updatesOpen, setUpdatesOpen] = useState(false);

  const counts = useAsync<OverviewCounts>(async () => {
    if (!project) {
      return { openTasks: 0, activeSessions: 0 };
    }
    const [tasks, sessions] = await Promise.all([
      api.tasks.list(project.id),
      api.agents.sessions(project.id),
    ]);
    return {
      openTasks: tasks.filter((task) => !["completed", "failed", "cancelled"].includes(task.state))
        .length,
      activeSessions: sessions.filter((session) =>
        ["starting", "ready", "running", "waiting_approval"].includes(session.state),
      ).length,
    };
  }, [project?.id]);

  const adapters = useAsync(() => api.agents.list(), []);

  const detected = adapters.data?.filter((adapter) => adapter.detected) ?? [];
  const recentProjects = projects.slice(0, 5);

  return (
    <>
      <PageHeader
        title={t("overview.title")}
        subtitle={t("overview.subtitle")}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => setUpdatesOpen(true)}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("overview.manage_updates")}
              {updateAvailable ? <Badge tone="accent">{updateAvailable}</Badge> : null}
            </button>
            <button type="button" className="primary" onClick={() => navigate("projects")}>
              <FolderOpen size={15} aria-hidden="true" />
              {t("projects.add")}
            </button>
          </>
        }
      />

      {updateAvailable ? (
        <Notice tone="info" title={t("updates.title")}>
          <p>{t("overview.update_available_badge", { version: updateAvailable })}</p>
          <button type="button" className="ghost small" onClick={() => setUpdatesOpen(true)}>
            {t("overview.manage_updates")}
          </button>
        </Notice>
      ) : null}

      <div className="overview-hero">
        <div>
          <p className="eyebrow">{t("overview.getting_started")}</p>
          <h2>{project ? project.name : t("overview.no_project")}</h2>
          <p className="muted">
            {project ? project.path : t("overview.no_project_hint")}
          </p>
          <div className="hero-actions">
            <button type="button" className="primary" onClick={() => navigate("projects")}>
              <FolderOpen size={15} aria-hidden="true" />
              {t("projects.add")}
            </button>
            <button type="button" className="ghost" onClick={() => navigate("diagnostics")}>
              <Monitor size={15} aria-hidden="true" />
              {t("overview.view_diagnostics")}
            </button>
          </div>
        </div>
        <div className="hero-status">
          <Badge tone={health?.core_online ? "success" : "danger"}>
            {health?.core_online ? t("status.online") : t("status.offline")}
          </Badge>
          {health ? <span className="mono small">v{health.version}</span> : null}
          {platform ? <span className="small muted">{platform.os_name}</span> : null}
        </div>
      </div>

      <div className="grid grid-4">
        <section className="card stat-card" aria-labelledby="overview-platform">
          <div className="card-icon">
            <Cpu size={18} aria-hidden="true" />
          </div>
          <h3 id="overview-platform">{t("overview.platform")}</h3>
          <p className="stat-value">{platform?.os_name ?? t("common.unknown")}</p>
          <dl className="stat-list">
            <div>
              <dt>{t("overview.kernel")}</dt>
              <dd>{platform?.kernel || t("common.unknown")}</dd>
            </div>
            <div>
              <dt>{t("overview.ram")}</dt>
              <dd>{platform ? formatBytes(platform.ram_mb * 1024 * 1024) : "—"}</dd>
            </div>
            <div>
              <dt>{t("overview.gpu")}</dt>
              <dd>{platform?.gpu ?? t("common.unknown")}</dd>
            </div>
          </dl>
        </section>

        <section className="card stat-card" aria-labelledby="overview-agents">
          <div className="card-icon">
            <Bot size={18} aria-hidden="true" />
          </div>
          <h3 id="overview-agents">{t("agents.adapters")}</h3>
          <p className="stat-value">
            {adapters.data
              ? t("overview.detected_agents", { count: detected.length })
              : t("common.loading")}
          </p>
          {adapters.data && detected.length === 0 ? (
            <p className="muted small">{t("overview.no_agents_detected")}</p>
          ) : (
            <ul className="plain-list">
              {detected.slice(0, 4).map((adapter) => (
                <li key={adapter.id}>{adapter.label}</li>
              ))}
            </ul>
          )}
          <button type="button" className="link-button" onClick={() => navigate("agents")}>
            {t("nav.agents")}
            <ArrowRight size={13} aria-hidden="true" />
          </button>
        </section>

        <section className="card stat-card" aria-labelledby="overview-tasks">
          <div className="card-icon">
            <ListChecks size={18} aria-hidden="true" />
          </div>
          <h3 id="overview-tasks">{t("nav.tasks")}</h3>
          {project ? (
            counts.loading ? (
              <LoadingRow label={t("common.loading")} />
            ) : (
              <>
                <p className="stat-value">
                  {t("overview.open_tasks_value", { count: counts.data?.openTasks ?? 0 })}
                </p>
                <p className="muted small">
                  {t("overview.active_agents_value", { count: counts.data?.activeSessions ?? 0 })}
                </p>
              </>
            )
          ) : (
            <p className="muted small">{t("overview.no_project")}</p>
          )}
          <button type="button" className="link-button" onClick={() => navigate("tasks")}>
            {t("nav.tasks")}
            <ArrowRight size={13} aria-hidden="true" />
          </button>
        </section>

        <section className="card stat-card" aria-labelledby="overview-resources">
          <div className="card-icon">
            <HardDrive size={18} aria-hidden="true" />
          </div>
          <h3 id="overview-resources">{t("overview.resources")}</h3>
          <p className="stat-value">
            {config ? config.resources.max_concurrent_agents : "—"}{" "}
            <span className="muted small">{t("settings.max_agents")}</span>
          </p>
          <p className="muted small">
            {config
              ? formatBytes(config.resources.max_output_bytes_per_session)
              : t("common.loading")}
          </p>
          <p className="muted small">
            {platform
              ? t("overview.recommended_agents") + " : " + platform.recommended_max_agents
              : ""}
          </p>
        </section>
      </div>

      {counts.error ? <ErrorBanner error={counts.error} onRetry={counts.reload} /> : null}
      {adapters.error ? <ErrorBanner error={adapters.error} onRetry={adapters.reload} /> : null}

      <div className="grid grid-2">
        <section className="card" aria-labelledby="overview-recent">
          <div className="section-heading">
            <h3 id="overview-recent">{t("overview.recent_projects")}</h3>
            <button type="button" className="link-button" onClick={() => navigate("projects")}>
              {t("common.more")}
              <ArrowRight size={13} aria-hidden="true" />
            </button>
          </div>
          {recentProjects.length === 0 ? (
            <p className="muted">{t("projects.no_projects")}</p>
          ) : (
            <ul className="project-mini-list">
              {recentProjects.map((entry) => (
                <li key={entry.id}>
                  <button
                    type="button"
                    className={project?.id === entry.id ? "project-mini active" : "project-mini"}
                    onClick={() => void selectProject(entry.id)}
                  >
                    <span className="project-mini-name">{entry.name}</span>
                    <span className="project-mini-path">{entry.path}</span>
                    {entry.is_git ? <Badge tone="info">{t("projects.is_git")}</Badge> : null}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="card" aria-labelledby="overview-safety">
          <div className="section-heading">
            <h3 id="overview-safety">{t("overview.safety")}</h3>
            <ShieldCheck size={18} aria-hidden="true" className="muted-icon" />
          </div>
          <p className="muted">{t("overview.safety_text")}</p>
          <button type="button" className="link-button" onClick={() => navigate("security")}>
            {t("nav.security")}
            <ArrowRight size={13} aria-hidden="true" />
          </button>
        </section>
      </div>

      <p className="runtime small muted">
        <span className="online-dot" aria-hidden="true" />
        {health ? `${health.app} · ${health.version} · ${health.platform}` : t("common.unknown")}
        <span aria-hidden="true"> · </span>
        {locale.toUpperCase()}
      </p>

      <UpdatesPanel open={updatesOpen} onClose={() => setUpdatesOpen(false)} />
    </>
  );
}
