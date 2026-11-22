import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  FileDiff,
  FolderPlus,
  GitBranch,
  GitCommitHorizontal,
  Minus,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import {
  api,
  toAppError,
  type AppError,
  type CommitInfo,
  type GitStatus,
  type Worktree,
} from "../lib/api";
import { formatDate, shortHash } from "../lib/format";
import { gitStatusRows, stagedRows, unstagedRows, type GitStatusRow } from "../lib/git";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";
import { Tabs, tabPanelId } from "../components/Tabs";

interface GitRepoData {
  status: GitStatus;
  branches: string[];
  log: CommitInfo[];
  worktrees: Worktree[];
}

type GitTab = "staged" | "unstaged";

function diffLineClass(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---")) {
    return "diff-meta";
  }
  if (line.startsWith("@@")) {
    return "diff-hunk";
  }
  if (line.startsWith("+")) {
    return "diff-add";
  }
  if (line.startsWith("-")) {
    return "diff-del";
  }
  if (line.startsWith("diff ") || line.startsWith("index ")) {
    return "diff-meta";
  }
  return "";
}

export function GitPage() {
  const { t, locale, project } = useApp();
  const repo = useAsync<GitRepoData | null>(async () => {
    if (!project) {
      return null;
    }
    const [status, branches, log, worktrees] = await Promise.all([
      api.git.status(project.id),
      api.git.branches(project.id),
      api.git.log(project.id, 30),
      api.git.worktrees(project.id),
    ]);
    return { status, branches, log, worktrees };
  }, [project?.id]);
  const fileAction = useAction();
  const commitAction = useAction();
  const worktreeAction = useAction();

  const [tab, setTab] = useState<GitTab>("staged");
  const [diff, setDiff] = useState<string | null>(null);
  const [diffPath, setDiffPath] = useState<string | null>(null);
  const [diffStaged, setDiffStaged] = useState<boolean | null>(null);
  const [diffError, setDiffError] = useState<AppError | null>(null);
  const [diffBusy, setDiffBusy] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  const [toRemove, setToRemove] = useState<Worktree | null>(null);
  const [forceRemove, setForceRemove] = useState(false);
  const [worktreeForm, setWorktreeForm] = useState({ target: "", branch: "", create: false });
  const [worktreeError, setWorktreeError] = useState<string | null>(null);

  useEffect(() => {
    setDiff(null);
    setDiffPath(null);
    setDiffStaged(null);
    setDiffError(null);
    setMessage(null);
    setCommitMessage("");
    setWorktreeForm({ target: "", branch: "", create: false });
  }, [project?.id]);

  const rows = repo.data ? gitStatusRows(repo.data.status) : [];
  const staged = stagedRows(rows);
  const unstaged = unstagedRows(rows);
  const activeRows = tab === "staged" ? staged : unstaged;

  const reloadAll = () => {
    repo.reload();
    setDiff(null);
    setDiffPath(null);
    setDiffStaged(null);
  };

  const loadDiff = async (stagedDiff: boolean, path: string | null) => {
    if (!project) {
      return;
    }
    setDiffBusy(true);
    setDiffError(null);
    try {
      const text = await api.git.diff(project.id, stagedDiff, path);
      setDiff(text);
      setDiffPath(path);
      setDiffStaged(stagedDiff);
    } catch (reason) {
      setDiffError(toAppError(reason));
    } finally {
      setDiffBusy(false);
    }
  };

  const handleFileAction = async (row: GitStatusRow) => {
    if (!project) {
      return;
    }
    setPendingPath(row.path);
    setMessage(null);
    const done = await fileAction.run(async () => {
      if (tab === "staged") {
        await api.git.unstage(project.id, [row.path]);
      } else {
        await api.git.stage(project.id, [row.path]);
      }
      return true;
    });
    setPendingPath(null);
    if (done) {
      reloadAll();
    }
  };

  const handleAll = async () => {
    if (!project) {
      return;
    }
    const paths = activeRows.map((row) => row.path);
    if (paths.length === 0) {
      return;
    }
    setMessage(null);
    const done = await fileAction.run(async () => {
      if (tab === "staged") {
        await api.git.unstage(project.id, paths);
      } else {
        await api.git.stage(project.id, paths);
      }
      return true;
    });
    if (done) {
      reloadAll();
    }
  };

  const handleCommit = async () => {
    if (!project) {
      return;
    }
    const hash = await commitAction.run(() => api.git.commit(project.id, commitMessage.trim()));
    if (hash) {
      setMessage(t("git.committed", { hash: shortHash(hash, 12) }));
      setCommitMessage("");
      reloadAll();
    }
  };

  const browseWorktree = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: t("git.worktree_path"),
      });
      if (typeof selected === "string") {
        setWorktreeForm((current) => ({ ...current, target: selected }));
      }
    } catch (reason) {
      worktreeAction.setError(toAppError(reason));
    }
  };

  const addWorktree = async () => {
    if (!project) {
      return;
    }
    const target = worktreeForm.target.trim();
    const branch = worktreeForm.branch.trim();
    if (!target.startsWith("/")) {
      setWorktreeError(t("git.worktree_absolute_required"));
      return;
    }
    if (!branch) {
      setWorktreeError(t("git.branch_required"));
      return;
    }
    setWorktreeError(null);
    const done = await worktreeAction.run(async () => {
      await api.git.addWorktree(project.id, target, branch, worktreeForm.create);
      return true;
    });
    if (done) {
      setMessage(t("git.worktree_added"));
      setWorktreeForm({ target: "", branch: "", create: false });
      reloadAll();
    }
  };

  const removeWorktree = async () => {
    if (!project || !toRemove) {
      return;
    }
    const target = toRemove;
    const done = await worktreeAction.run(async () => {
      await api.git.removeWorktree(project.id, target.path, forceRemove);
      return true;
    });
    if (done) {
      setMessage(t("git.worktree_removed"));
      setToRemove(null);
      setForceRemove(false);
      reloadAll();
    }
  };

  if (!project) {
    return (
      <>
        <PageHeader title={t("git.title")} />
        <ProjectGate title={t("git.title")} />
      </>
    );
  }

  const notRepo =
    repo.error?.code === "unsupported" &&
    !repo.error.message.toLowerCase().includes("binaire");

  return (
    <>
      <PageHeader
        title={t("git.title")}
        subtitle={project.path}
        actions={
          <button type="button" className="ghost" onClick={reloadAll} disabled={repo.loading}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("git.status_refresh")}
          </button>
        }
      />

      {message ? <Notice tone="success">{message}</Notice> : null}

      {repo.loading ? (
        <LoadingRow label={t("common.loading")} />
      ) : notRepo ? (
        <EmptyState
          icon={<GitBranch size={22} aria-hidden="true" />}
          title={t("git.not_a_repo")}
          description={
            <>
              <p>{t("git.not_a_repo_hint")}</p>
              <p className="mono small">{repo.error?.message}</p>
            </>
          }
          action={
            <button type="button" className="ghost" onClick={repo.reload}>
              {t("common.retry")}
            </button>
          }
        />
      ) : repo.error ? (
        <ErrorBanner error={repo.error} onRetry={repo.reload} />
      ) : repo.data ? (
        <>
          <section className="card git-summary" aria-labelledby="git-status">
            <div className="section-heading">
              <h2 id="git-status">{t("git.status")}</h2>
              <div className="inline-status">
                <Badge tone="accent">
                  <GitBranch size={12} aria-hidden="true" />
                  {repo.data.status.branch || "HEAD"}
                </Badge>
                <Badge tone={repo.data.status.clean ? "success" : "warning"}>
                  {repo.data.status.clean
                    ? t("git.clean")
                    : t("git.changes_count", { count: repo.data.status.entries.length })}
                </Badge>
                {repo.data.status.ahead > 0 || repo.data.status.behind > 0 ? (
                  <Badge tone="info">
                    {t("git.ahead_behind", {
                      ahead: repo.data.status.ahead,
                      behind: repo.data.status.behind,
                    })}
                  </Badge>
                ) : null}
              </div>
            </div>
            <p className="muted small">
              {t("git.current_branch")} : {repo.data.status.branch}
            </p>
          </section>

          <div className="git-workspace">
            <section className="card" aria-labelledby="git-changes">
              <div className="section-heading">
                <h2 id="git-changes">{t("git.changes")}</h2>
                <button
                  type="button"
                  className="ghost small"
                  onClick={() => void handleAll()}
                  disabled={activeRows.length === 0 || fileAction.busy}
                >
                  {tab === "staged" ? (
                    <Minus size={13} aria-hidden="true" />
                  ) : (
                    <Plus size={13} aria-hidden="true" />
                  )}
                  {tab === "staged" ? t("git.unstage_all") : t("git.stage_all")}
                </button>
              </div>

              <Tabs
                idPrefix="git"
                label={t("git.changes")}
                active={tab}
                onChange={(next) => {
                  setTab(next);
                  setDiff(null);
                  setDiffStaged(null);
                  setDiffPath(null);
                }}
                tabs={[
                  { id: "staged", label: t("git.staged"), detail: String(staged.length) },
                  { id: "unstaged", label: t("git.unstaged"), detail: String(unstaged.length) },
                ]}
              />

              {fileAction.error ? <ErrorBanner error={fileAction.error} /> : null}
              <div
                role="tabpanel"
                id={tabPanelId("git", tab)}
                aria-labelledby={`git-tab-${tab}`}
                className="tab-panel"
              >
                {activeRows.length === 0 ? (
                  <p className="muted">{t("git.no_changes")}</p>
                ) : (
                  <ul className="change-list">
                    {activeRows.map((row) => {
                      const kind = tab === "staged" ? row.stagedKind : row.worktreeKind;
                      return (
                        <li key={`${row.path}-${tab}`} className="change-row">
                          <span className="change-code" aria-hidden="true">
                            {tab === "staged" ? row.stagedCode || "•" : row.worktreeCode || "•"}
                          </span>
                          <span className="change-path mono" title={row.path}>
                            {row.path}
                          </span>
                          <Badge tone={kind === "deleted" ? "danger" : "neutral"}>
                            {t(`git.change_${kind}`)}
                          </Badge>
                          <div className="change-actions">
                            <button
                              type="button"
                              className="ghost small"
                              onClick={() => void loadDiff(tab === "staged", row.path)}
                              disabled={diffBusy}
                            >
                              <FileDiff size={13} aria-hidden="true" />
                              {t("git.diff")}
                            </button>
                            <button
                              type="button"
                              className="ghost small"
                              onClick={() => void handleFileAction(row)}
                              disabled={fileAction.busy}
                              aria-busy={pendingPath === row.path}
                            >
                              {pendingPath === row.path ? <Spinner /> : null}
                              {tab === "staged" ? t("git.unstage") : t("git.stage")}
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>

              <div className="commit-box">
                <Field
                  label={t("git.commit_message")}
                  htmlFor="git-commit-message"
                  hint={t("git.commit_hint")}
                >
                  <textarea
                    id="git-commit-message"
                    className="input textarea"
                    rows={3}
                    value={commitMessage}
                    onChange={(event) => setCommitMessage(event.target.value)}
                  />
                </Field>
                <button
                  type="button"
                  className="primary"
                  onClick={() => void handleCommit()}
                  disabled={
                    commitAction.busy || staged.length === 0 || commitMessage.trim().length === 0
                  }
                  aria-busy={commitAction.busy}
                >
                  {commitAction.busy ? (
                    <Spinner />
                  ) : (
                    <GitCommitHorizontal size={14} aria-hidden="true" />
                  )}
                  {t("git.commit")}
                </button>
                {staged.length === 0 ? (
                  <p className="muted small">{t("git.commit_disabled")}</p>
                ) : null}
                {commitAction.error ? <ErrorBanner error={commitAction.error} /> : null}
              </div>
            </section>

            <section className="card diff-card" aria-labelledby="git-diff">
              <div className="section-heading">
                <h2 id="git-diff">{t("git.diff")}</h2>
                <div className="inline-status">
                  {diffStaged !== null ? (
                    <Badge tone={diffStaged ? "accent" : "warning"}>
                      {diffStaged ? t("git.staged") : t("git.unstaged")}
                    </Badge>
                  ) : null}
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void loadDiff(tab === "staged", null)}
                    disabled={diffBusy}
                    aria-busy={diffBusy}
                  >
                    {diffBusy ? <Spinner /> : <FileDiff size={13} aria-hidden="true" />}
                    {t("git.all_changes")}
                  </button>
                </div>
              </div>
              {diffError ? <ErrorBanner error={diffError} /> : null}
              {diff === null ? (
                <p className="muted">{t("git.diff_hint")}</p>
              ) : diff.trim().length === 0 ? (
                <p className="muted">{t("git.diff_empty")}</p>
              ) : (
                <>
                  <p className="mono small">
                    {diffPath ?? t("git.all_changes")} · {diffStaged ? t("git.staged") : t("git.unstaged")}
                  </p>
                  <pre className="diff-view">
                    {diff.split("\n").map((line, index) => (
                      <span key={index} className={diffLineClass(line)}>
                        {line}
                        {"\n"}
                      </span>
                    ))}
                  </pre>
                </>
              )}
            </section>
          </div>

          <div className="grid grid-2">
            <section className="card" aria-labelledby="git-branches">
              <div className="section-heading">
                <h2 id="git-branches">{t("git.branches")}</h2>
              </div>
              <ul className="plain-list branch-list">
                {repo.data.branches.map((branch) => (
                  <li key={branch}>
                    <GitBranch size={13} aria-hidden="true" />
                    <span className={branch === repo.data?.status.branch ? "strong" : undefined}>
                      {branch}
                    </span>
                    {branch === repo.data?.status.branch ? (
                      <Badge tone="accent">{t("git.current_branch")}</Badge>
                    ) : null}
                  </li>
                ))}
              </ul>
            </section>

            <section className="card" aria-labelledby="git-log">
              <div className="section-heading">
                <h2 id="git-log">{t("git.log")}</h2>
              </div>
              {repo.data.log.length === 0 ? (
                <p className="muted">{t("git.no_commits")}</p>
              ) : (
                <div className="table-wrap">
                  <table className="table">
                    <thead>
                      <tr>
                        <th scope="col">{t("git.hash")}</th>
                        <th scope="col">{t("git.subject")}</th>
                        <th scope="col">{t("git.author")}</th>
                        <th scope="col">{t("git.date")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {repo.data.log.map((commit) => (
                        <tr key={commit.hash}>
                          <td className="mono" title={commit.hash}>
                            {shortHash(commit.hash, 9)}
                          </td>
                          <td>{commit.subject}</td>
                          <td>{commit.author}</td>
                          <td>{formatDate(commit.date, locale)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          </div>

          <section className="card" aria-labelledby="git-worktrees">
            <div className="section-heading">
              <h2 id="git-worktrees">{t("git.worktrees")}</h2>
            </div>
            {worktreeAction.error ? <ErrorBanner error={worktreeAction.error} /> : null}
            {repo.data.worktrees.length === 0 ? (
              <p className="muted">{t("git.no_worktrees")}</p>
            ) : (
              <ul className="worktree-list">
                {repo.data.worktrees.map((worktree, index) => (
                  <li key={worktree.path} className="worktree-row">
                    <div>
                      <p className="mono">{worktree.path}</p>
                      <p className="small muted">
                        {worktree.branch ?? "—"} · {t("git.hash")} {shortHash(worktree.head, 9)}
                        {index === 0 ? ` · ${t("git.main_worktree")}` : ""}
                      </p>
                    </div>
                    {index > 0 ? (
                      <button
                        type="button"
                        className="ghost small danger-text"
                        onClick={() => {
                          worktreeAction.reset();
                          setForceRemove(false);
                          setToRemove(worktree);
                        }}
                        disabled={worktreeAction.busy}
                      >
                        <Trash2 size={13} aria-hidden="true" />
                        {t("git.remove_worktree")}
                      </button>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}

            <div className="worktree-form">
              <h3 className="section-title">{t("git.add_worktree")}</h3>
              <div className="form-grid">
                <Field
                  label={t("git.worktree_path")}
                  htmlFor="worktree-target"
                  hint={t("git.worktree_absolute_required")}
                  error={worktreeError}
                >
                  <div className="input-row">
                    <input
                      id="worktree-target"
                      className="input mono"
                      value={worktreeForm.target}
                      onChange={(event) => {
                        setWorktreeError(null);
                        setWorktreeForm({ ...worktreeForm, target: event.target.value });
                      }}
                      placeholder={t("git.worktree_placeholder")}
                    />
                    <button type="button" className="ghost small" onClick={() => void browseWorktree()}>
                      <FolderPlus size={13} aria-hidden="true" />
                      {t("git.browse_target")}
                    </button>
                  </div>
                </Field>
                <Field label={t("git.worktree_branch")} htmlFor="worktree-branch" required>
                  <input
                    id="worktree-branch"
                    className="input mono"
                    list="git-branch-list"
                    value={worktreeForm.branch}
                    onChange={(event) => {
                      setWorktreeError(null);
                      setWorktreeForm({ ...worktreeForm, branch: event.target.value });
                    }}
                  />
                  <datalist id="git-branch-list">
                    {repo.data.branches.map((branch) => (
                      <option key={branch} value={branch} />
                    ))}
                  </datalist>
                </Field>
              </div>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={worktreeForm.create}
                  onChange={(event) =>
                    setWorktreeForm({ ...worktreeForm, create: event.target.checked })
                  }
                />
                <span>{t("git.create_branch")}</span>
              </label>
              <div className="button-row">
                <button
                  type="button"
                  className="primary small"
                  onClick={() => void addWorktree()}
                  disabled={
                    worktreeAction.busy ||
                    worktreeForm.target.trim().length === 0 ||
                    worktreeForm.branch.trim().length === 0
                  }
                  aria-busy={worktreeAction.busy}
                >
                  {worktreeAction.busy ? <Spinner /> : <Plus size={13} aria-hidden="true" />}
                  {t("git.add_worktree")}
                </button>
              </div>
            </div>
          </section>
        </>
      ) : null}

      <ConfirmDialog
        open={toRemove !== null}
        title={t("git.remove_confirm_title", { path: toRemove?.path ?? "" })}
        description={<p>{t("git.remove_confirm_text")}</p>}
        confirmLabel={t("git.remove_worktree")}
        onCancel={() => setToRemove(null)}
        onConfirm={() => void removeWorktree()}
        busy={worktreeAction.busy}
        danger
      >
        <label className="checkbox-row">
          <input
            type="checkbox"
            checked={forceRemove}
            onChange={(event) => setForceRemove(event.target.checked)}
          />
          <span>{t("git.force")}</span>
        </label>
      </ConfirmDialog>
    </>
  );
}
