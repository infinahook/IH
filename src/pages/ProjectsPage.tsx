import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, GitBranch, Plus, Trash2 } from "lucide-react";
import { api, toAppError, type ProjectView } from "../lib/api";
import { formatDate } from "../lib/format";
import { useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";

export function ProjectsPage() {
  const {
    t,
    locale,
    projects,
    projectsLoading,
    project,
    addProject,
    removeProject,
    selectProject,
    refreshProjects,
  } = useApp();
  const add = useAction();
  const remove = useAction();
  const openFolder = useAction();
  const [message, setMessage] = useState<string | null>(null);
  const [toRemove, setToRemove] = useState<ProjectView | null>(null);

  const handleAdd = async () => {
    setMessage(null);
    let selected: string | string[] | null = null;
    try {
      selected = await open({
        directory: true,
        multiple: false,
        title: t("projects.dialog_title"),
      });
    } catch (reason) {
      add.setError(toAppError(reason));
      return;
    }
    if (typeof selected !== "string") {
      return;
    }
    const normalized = selected.replace(/\/+$/, "");
    const existing = projects.find(
      (entry) => entry.path === selected || entry.path === normalized,
    );
    if (existing) {
      await selectProject(existing.id);
      setMessage(t("projects.already_added"));
      return;
    }
    const added = await add.run(() => addProject(selected));
    if (added) {
      setMessage(t("projects.added", { name: added.name }));
    }
  };

  const handleRemove = async () => {
    if (!toRemove) {
      return;
    }
    const target = toRemove;
    const done = await remove.run(async () => {
      await removeProject(target.id);
      return true;
    });
    if (done) {
      setToRemove(null);
      setMessage(t("projects.removed"));
    }
  };

  return (
    <>
      <PageHeader
        title={t("projects.title")}
        subtitle={t("projects.add_hint")}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => void refreshProjects()}>
              {t("common.refresh")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void handleAdd()}
              disabled={add.busy}
              aria-busy={add.busy}
            >
              <Plus size={15} aria-hidden="true" />
              {t("projects.add")}
            </button>
          </>
        }
      />

      {add.error ? <ErrorBanner error={add.error} /> : null}
      {remove.error ? <ErrorBanner error={remove.error} /> : null}
      {openFolder.error ? <ErrorBanner error={openFolder.error} /> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}
      <Notice tone="info">{t("projects.remove_hint")}</Notice>

      {projectsLoading ? (
        <LoadingRow label={t("common.loading")} />
      ) : projects.length === 0 ? (
        <EmptyState
          icon={<FolderOpen size={22} aria-hidden="true" />}
          title={t("projects.no_projects")}
          description={<p>{t("projects.add_hint")}</p>}
          action={
            <button
              type="button"
              className="primary"
              onClick={() => void handleAdd()}
              disabled={add.busy}
            >
              {t("projects.add")}
            </button>
          }
        />
      ) : (
        <ul className="project-list">
          {projects.map((entry) => {
            const selected = project?.id === entry.id;
            return (
              <li
                key={entry.id}
                className={selected ? "project-row selected" : "project-row"}
                aria-current={selected ? "true" : undefined}
              >
                <div className="project-row-main">
                  <div className="project-row-title">
                    <h2>{entry.name}</h2>
                    {selected ? <Badge tone="accent">{t("projects.current")}</Badge> : null}
                    {entry.is_git ? (
                      <Badge tone="info">
                        <GitBranch size={11} aria-hidden="true" />
                        {t("projects.is_git")}
                      </Badge>
                    ) : (
                      <Badge>{t("projects.not_git")}</Badge>
                    )}
                  </div>
                  <p className="mono small project-path">{entry.path}</p>
                  <p className="muted small">
                    {t("projects.last_opened")} : {formatDate(entry.last_opened_at, locale)}
                  </p>
                </div>
                <div className="project-row-actions">
                  <button
                    type="button"
                    className={selected ? "primary small" : "ghost small"}
                    onClick={() => void selectProject(entry.id)}
                    disabled={selected}
                  >
                    {selected ? t("projects.selected") : t("common.select")}
                  </button>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => {
                      void openFolder.run(() => api.openPath(entry.path));
                    }}
                  >
                    <FolderOpen size={14} aria-hidden="true" />
                    {t("projects.open_folder")}
                  </button>
                  <button
                    type="button"
                    className="ghost small danger-text"
                    onClick={() => {
                      remove.reset();
                      setToRemove(entry);
                    }}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                    {t("projects.remove_from_list")}
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <ConfirmDialog
        open={toRemove !== null}
        title={t("projects.remove_confirm_title", { name: toRemove?.name ?? "" })}
        description={<p>{t("projects.remove_confirm_text")}</p>}
        confirmLabel={t("projects.remove_from_list")}
        onCancel={() => setToRemove(null)}
        onConfirm={() => void handleRemove()}
        busy={remove.busy}
        danger
      />
    </>
  );
}
