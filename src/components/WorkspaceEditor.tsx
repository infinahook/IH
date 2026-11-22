import { useState, type KeyboardEvent } from "react";
import {
  AlertTriangle,
  FileCode2,
  FileDiff,
  RefreshCw,
  Save,
  X,
} from "lucide-react";
import { LANGUAGE_LABELS, languageIdForPath } from "../lib/languages";
import { formatBytes, shortHash } from "../lib/format";
import { useApp } from "../lib/store";
import { fileTitle, isTabDirty } from "../lib/tabs";
import type { WorkspaceController } from "../lib/useWorkspace";
import { Badge } from "./Badge";
import { CodeMirrorEditor } from "./CodeMirrorEditor";
import { ConfirmDialog } from "./ConfirmDialog";
import { DiffView } from "./DiffView";
import { EmptyState, Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";
import { Notice } from "./Notice";

/**
 * Panneau droit : onglets de fichiers, éditeur CodeMirror 6 et vue de diff.
 * Toutes les opérations passent par le contrôleur `useWorkspace` ; aucun
 * écrasement disque n'a lieu sans `workspace_save`/`diff_accept`.
 */
export function WorkspaceEditor({ workspace }: { workspace: WorkspaceController }) {
  const { t } = useApp();
  const [forceOpen, setForceOpen] = useState(false);
  const {
    tabs,
    activePath,
    activeTab,
    dirty,
    diff,
    diffError,
    diffBusy,
    saveBusy,
    fileBusy,
    saveError,
    saveConflict,
  } = workspace;

  const language = activeTab ? languageIdForPath(activeTab.path) : "plain";
  const activeIndex = tabs.findIndex((tab) => tab.path === activePath);

  const moveTab = (event: KeyboardEvent<HTMLButtonElement>, delta: number) => {
    if (tabs.length === 0) {
      return;
    }
    const current = Math.max(activeIndex, 0);
    const next = (current + delta + tabs.length) % tabs.length;
    const target = tabs[next];
    if (!target) {
      return;
    }
    event.preventDefault();
    workspace.activate(target.path);
    window.requestAnimationFrame(() => {
      document.getElementById(`ws-file-tab-${next}`)?.focus();
    });
  };

  const panelId = "workspace-editor-panel";

  return (
    <section className="card workspace-editor" aria-label={t("workspace.editor")}>
      <div className="workspace-editor-tabs" role="tablist" aria-label={t("workspace.editor_tabs")}>
        {tabs.length === 0 ? (
          <span className="small muted workspace-tabs-empty">{t("workspace.no_open_tabs")}</span>
        ) : null}
        {tabs.map((tab, index) => {
          const selected = tab.path === activePath;
          const modified = isTabDirty(tab);
          return (
            <div key={tab.path} className={selected ? "file-tab active" : "file-tab"}>
              <button
                type="button"
                role="tab"
                id={`ws-file-tab-${index}`}
                aria-selected={selected}
                aria-controls={panelId}
                tabIndex={selected ? 0 : -1}
                className="file-tab-button"
                title={tab.path}
                onClick={() => {
                  workspace.closeDiff();
                  workspace.activate(tab.path);
                }}
                onKeyDown={(event) => {
                  if (event.key === "ArrowRight") {
                    moveTab(event, 1);
                  } else if (event.key === "ArrowLeft") {
                    moveTab(event, -1);
                  }
                }}
              >
                {modified ? (
                  <span
                    className="file-tab-dirty"
                    aria-label={t("workspace.unsaved")}
                    title={t("workspace.unsaved")}
                  />
                ) : null}
                <span className="file-tab-title">{tab.title}</span>
                {tab.readOnly ? <Badge tone="warning">{t("editor.read_only")}</Badge> : null}
              </button>
              <button
                type="button"
                className="file-tab-close"
                aria-label={t("workspace.close_tab", { title: tab.title })}
                title={t("workspace.close_tab", { title: tab.title })}
                onClick={() => workspace.requestClose(tab.path)}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>

      {diff ? (
        <div
          role="tabpanel"
          id={panelId}
          className="workspace-editor-panel"
        >
          <DiffView
            preview={diff}
            busy={diffBusy}
            error={diffError}
            onAccept={() => void workspace.acceptDiff()}
            onReject={() => void workspace.rejectDiff()}
            onRetryPrepare={() => void workspace.showDiff()}
            onClose={workspace.closeDiff}
            errorIsConflict={diffError?.code === "conflict"}
          />
        </div>
      ) : (
        <div
          role="tabpanel"
          id={panelId}
          aria-labelledby={activeIndex >= 0 ? `ws-file-tab-${activeIndex}` : undefined}
          className="workspace-editor-panel"
        >
          <div className="workspace-editor-toolbar">
            {activeTab ? (
              <>
                <span className="mono small workspace-editor-path" title={activeTab.path}>
                  {activeTab.path}
                </span>
                <span className="small muted">{formatBytes(activeTab.size)}</span>
                <Badge tone="neutral">{LANGUAGE_LABELS[language]}</Badge>
                {dirty ? <Badge tone="warning">{t("workspace.unsaved")}</Badge> : null}
                {activeTab.sha256 ? (
                  <span className="small muted mono" title={activeTab.sha256}>
                    {shortHash(activeTab.sha256)}
                  </span>
                ) : null}
              </>
            ) : (
              <span className="small muted">{t("workspace.no_open_tabs")}</span>
            )}
            <div className="button-row compact workspace-editor-actions">
              <button
                type="button"
                className="ghost small"
                onClick={() => void workspace.reloadActive()}
                disabled={!activeTab || fileBusy || saveBusy}
                aria-busy={fileBusy}
                title={t("workspace.reload")}
              >
                {fileBusy ? <Spinner /> : <RefreshCw size={13} aria-hidden="true" />}
                {t("workspace.reload")}
              </button>
              <button
                type="button"
                className="ghost small"
                onClick={() => void workspace.showDiff()}
                disabled={!activeTab || !dirty || diffBusy}
                aria-busy={diffBusy}
                title={t("workspace.show_diff")}
              >
                {diffBusy ? <Spinner /> : <FileDiff size={13} aria-hidden="true" />}
                {t("workspace.show_diff")}
              </button>
              <button
                type="button"
                className="primary small"
                onClick={() => void workspace.save()}
                disabled={!activeTab || !dirty || saveBusy || activeTab.readOnly}
                aria-busy={saveBusy}
                title={t("workspace.save_shortcut")}
              >
                {saveBusy ? <Spinner /> : <Save size={13} aria-hidden="true" />}
                {t("common.save")}
              </button>
            </div>
          </div>

          {workspace.fileError ? (
            <ErrorBanner error={workspace.fileError} onDismiss={workspace.dismissFileError} />
          ) : null}
          {saveError ? (
            <ErrorBanner error={saveError} onDismiss={workspace.dismissSaveError} />
          ) : null}
          {saveConflict ? (
            <div className="banner banner-error" role="alert">
              <AlertTriangle size={18} aria-hidden="true" />
              <div className="banner-content">
                <strong>{t("workspace.conflict_title")}</strong>
                <p className="banner-detail">
                  {t("workspace.conflict_text", { path: saveConflict.path })}
                </p>
                <p className="banner-meta">
                  <span>{t("errors.code_label", { code: saveConflict.error.code })}</span>
                </p>
              </div>
              <div className="banner-actions">
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void workspace.reloadActive()}
                  disabled={fileBusy || saveBusy}
                >
                  <RefreshCw size={14} aria-hidden="true" />
                  {t("workspace.reload_from_disk")}
                </button>
                <button
                  type="button"
                  className="danger"
                  onClick={() => setForceOpen(true)}
                  disabled={saveBusy}
                >
                  <Save size={14} aria-hidden="true" />
                  {t("workspace.force_save")}
                </button>
              </div>
            </div>
          ) : null}
          {workspace.notice ? (
            <div className="workspace-notice-row">
              <Notice tone={workspace.notice.tone}>{workspace.notice.text}</Notice>
              <button
                type="button"
                className="icon-button"
                aria-label={t("common.close")}
                onClick={workspace.dismissNotice}
              >
                <X size={14} />
              </button>
            </div>
          ) : null}

          <div className="workspace-editor-body">
            {activeTab ? (
              <CodeMirrorEditor
                key={activeTab.path}
                path={activeTab.path}
                value={activeTab.content}
                readOnly={activeTab.readOnly}
                ariaLabel={t("editor.content_label")}
                onChange={(next) => workspace.updateContent(activeTab.path, next)}
                onSave={() => void workspace.save()}
              />
            ) : (
              <EmptyState
                icon={<FileCode2 size={22} aria-hidden="true" />}
                title={t("workspace.select_file")}
                description={<p>{t("workspace.select_file_hint")}</p>}
              />
            )}
          </div>
        </div>
      )}

      <ConfirmDialog
        open={forceOpen}
        title={t("workspace.force_save_title")}
        description={
          <>
            <p>{t("workspace.force_save_text")}</p>
            <p className="small muted">{t("workspace.force_save_hint")}</p>
          </>
        }
        confirmLabel={t("workspace.force_save_confirm")}
        onCancel={() => setForceOpen(false)}
        onConfirm={() => {
          setForceOpen(false);
          void workspace.save(true, saveConflict?.path ?? null);
        }}
        busy={saveBusy}
        danger
      />

      <ConfirmDialog
        open={workspace.closeTarget !== null}
        title={t("workspace.close_dirty_title", {
          title: workspace.closeTarget ? fileTitle(workspace.closeTarget) : "",
        })}
        description={<p>{t("workspace.close_dirty_text")}</p>}
        confirmLabel={t("workspace.close_discard")}
        onCancel={workspace.cancelClose}
        onConfirm={workspace.confirmClose}
        danger
      />
    </section>
  );
}
