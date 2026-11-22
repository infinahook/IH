import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ChevronDown, ChevronRight, FileCode2, Folder, RefreshCw, Save, Search, X } from "lucide-react";
import { api, toAppError, type AppError, type FileContent, type SearchHit } from "../lib/api";
import { buildFileTree, lineOffset, type TreeNode } from "../lib/fs";
import { countLines, formatBytes } from "../lib/format";
import { useAction, useAsync } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";
import { Tabs, tabPanelId } from "../components/Tabs";

type EditorTab = "tree" | "search";

const MAX_EDITOR_BYTES = 1_000_000;

export function EditorPage() {
  const { t, project } = useApp();
  const tree = useAsync(async () => {
    if (!project) {
      return [];
    }
    return api.files.tree(project.id, 4);
  }, [project?.id]);
  const save = useAction();
  const [tab, setTab] = useState<EditorTab>("tree");
  const [file, setFile] = useState<FileContent | null>(null);
  const [draft, setDraft] = useState("");
  const [fileError, setFileError] = useState<AppError | null>(null);
  const [loadingFile, setLoadingFile] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchError, setSearchError] = useState<AppError | null>(null);
  const [pendingLine, setPendingLine] = useState<number | null>(null);
  const [pendingChange, setPendingChange] = useState<
    { kind: "open"; path: string; line: number | null } | { kind: "close" } | null
  >(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLPreElement>(null);
  const openToken = useRef(0);
  const projectIdRef = useRef(project?.id);
  projectIdRef.current = project?.id;

  const nodes = useMemo(() => buildFileTree(tree.data ?? []), [tree.data]);
  const dirty = file !== null && draft !== file.content;
  const truncated = file?.truncated === true;
  const lineCount = draft.length === 0 ? 1 : countLines(draft);

  useEffect(() => {
    openToken.current += 1;
    setFile(null);
    setDraft("");
    setFileError(null);
    setLoadingFile(false);
    setHits(null);
    setQuery("");
    setSearchBusy(false);
    setSearchError(null);
  }, [project?.id]);

  useEffect(() => {
    if (pendingLine === null || !file || !textareaRef.current) {
      return;
    }
    const offset = lineOffset(draft, pendingLine);
    const textarea = textareaRef.current;
    textarea.focus();
    textarea.setSelectionRange(offset, offset);
    textarea.scrollTop = Math.max(0, (pendingLine - 3) * 20);
    if (gutterRef.current) {
      gutterRef.current.scrollTop = textarea.scrollTop;
    }
    setPendingLine(null);
  }, [pendingLine, file, draft]);

  const openFile = async (path: string, line: number | null = null) => {
    if (!project) {
      return;
    }
    const scopeId = project.id;
    const token = openToken.current + 1;
    openToken.current = token;
    setLoadingFile(true);
    setFileError(null);
    setMessage(null);
    try {
      const content = await api.files.read(project.id, path, MAX_EDITOR_BYTES);
      if (openToken.current !== token || projectIdRef.current !== scopeId) {
        return;
      }
      setFile(content);
      setDraft(content.content);
      setPendingLine(line);
    } catch (reason) {
      if (openToken.current === token && projectIdRef.current === scopeId) {
        setFileError(toAppError(reason));
      }
    } finally {
      if (openToken.current === token) {
        setLoadingFile(false);
      }
    }
  };

  const closeFile = () => {
    setFile(null);
    setDraft("");
    setFileError(null);
  };

  const requestOpen = (path: string, line: number | null = null) => {
    if (dirty) {
      setPendingChange({ kind: "open", path, line });
      return;
    }
    void openFile(path, line);
  };

  const requestClose = () => {
    if (dirty) {
      setPendingChange({ kind: "close" });
      return;
    }
    closeFile();
  };

  const confirmDiscard = async () => {
    const action = pendingChange;
    setPendingChange(null);
    if (!action) {
      return;
    }
    if (action.kind === "close") {
      closeFile();
    } else {
      await openFile(action.path, action.line);
    }
  };

  const saveFile = async () => {
    if (!project || !file || truncated || !dirty || save.busy) {
      return;
    }
    const written = await save.run(() => api.files.write(project.id, file.path, draft));
    if (written !== undefined) {
      setFile({ ...file, content: draft, size: written });
      setMessage(t("editor.saved_msg", { bytes: formatBytes(written) }));
    }
  };

  const handleEditorKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void saveFile();
    }
  };

  const runSearch = async () => {
    if (!project || query.trim().length === 0) {
      return;
    }
    const scopeId = project.id;
    setSearchBusy(true);
    setSearchError(null);
    try {
      const found = await api.files.search(project.id, query.trim(), 100);
      if (projectIdRef.current === scopeId) {
        setHits(found);
      }
    } catch (reason) {
      if (projectIdRef.current === scopeId) {
        setSearchError(toAppError(reason));
      }
    } finally {
      if (projectIdRef.current === scopeId) {
        setSearchBusy(false);
      }
    }
  };

  const renderNodes = (entries: TreeNode[], depth: number): ReactNode =>
    entries.map((entry) =>
      entry.node.is_dir ? (
        <details key={entry.node.path} className="tree-folder" open={depth < 2}>
          <summary>
            <ChevronDown size={12} className="tree-chevron open" aria-hidden="true" />
            <ChevronRight size={12} className="tree-chevron closed" aria-hidden="true" />
            <Folder size={13} aria-hidden="true" />
            <span>{entry.node.name}</span>
          </summary>
          <div className="tree-children">{renderNodes(entry.children, depth + 1)}</div>
        </details>
      ) : (
        <button
          key={entry.node.path}
          type="button"
          className={file?.path === entry.node.path ? "tree-file active" : "tree-file"}
          onClick={() => requestOpen(entry.node.path)}
          title={entry.node.path}
        >
          <FileCode2 size={13} aria-hidden="true" />
          <span>{entry.node.name}</span>
        </button>
      ),
    );

  if (!project) {
    return (
      <>
        <PageHeader title={t("editor.title")} />
        <ProjectGate title={t("editor.title")} />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={t("editor.title")}
        subtitle={project.path}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => tree.reload()} disabled={tree.loading}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            {file ? (
              <span className="mono small">
                {file.path} · {formatBytes(file.size)} · {t("editor.lines")} {lineCount}
              </span>
            ) : null}
            <button
              type="button"
              className="primary"
              onClick={() => void saveFile()}
              disabled={!file || !dirty || truncated || save.busy}
              aria-busy={save.busy}
            >
              {save.busy ? <Spinner /> : <Save size={15} aria-hidden="true" />}
              {t("editor.save_file")}
            </button>
          </>
        }
      />

      {tree.error ? <ErrorBanner error={tree.error} onRetry={tree.reload} /> : null}
      {fileError ? <ErrorBanner error={fileError} /> : null}
      {save.error ? <ErrorBanner error={save.error} /> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}

      <div className="editor-layout">
        <aside className="card editor-sidebar">
          <Tabs
            idPrefix="editor"
            label={t("editor.project_files")}
            active={tab}
            onChange={setTab}
            tabs={[
              { id: "tree", label: t("editor.tree") },
              { id: "search", label: t("common.search") },
            ]}
          />
          <div
            role="tabpanel"
            id={tabPanelId("editor", tab)}
            aria-labelledby={`editor-tab-${tab}`}
            className="tab-panel"
          >
            {tab === "tree" ? (
              tree.loading ? (
                <LoadingRow label={t("common.loading")} />
              ) : nodes.length === 0 ? (
                <p className="muted">{t("editor.no_files")}</p>
              ) : (
                <div className="tree" aria-label={t("editor.tree_label")}>
                  {renderNodes(nodes, 0)}
                </div>
              )
            ) : (
              <div className="search-panel">
                <div className="input-row">
                  <input
                    className="input"
                    value={query}
                    placeholder={t("editor.search_placeholder")}
                    aria-label={t("editor.search_placeholder")}
                    onChange={(event) => setQuery(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void runSearch();
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => void runSearch()}
                    disabled={searchBusy || query.trim().length === 0}
                    aria-busy={searchBusy}
                  >
                    {searchBusy ? <Spinner /> : <Search size={13} aria-hidden="true" />}
                    {t("common.search")}
                  </button>
                </div>
                <p className="muted small">{t("editor.search_hint")}</p>
                {searchError ? <ErrorBanner error={searchError} /> : null}
                {hits === null ? null : hits.length === 0 ? (
                  <p className="muted">{t("editor.no_hits")}</p>
                ) : (
                  <>
                    <p className="small muted">{t("editor.results", { count: hits.length })}</p>
                    <ul className="search-hits">
                      {hits.map((hit) => (
                        <li key={`${hit.path}-${hit.line}-${hit.column}`}>
                          <button
                            type="button"
                            className="search-hit"
                            onClick={() => requestOpen(hit.path, hit.line)}
                            title={t("editor.open_hit")}
                          >
                            <span className="mono small">
                              {hit.path}:{hit.line}
                            </span>
                            <span className="small muted">
                              {t("editor.hit", { line: hit.line, column: hit.column })}
                            </span>
                            <span className="search-preview mono">{hit.preview}</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  </>
                )}
              </div>
            )}
          </div>
        </aside>

        <section className="card editor-main" aria-labelledby="editor-file">
          {loadingFile ? (
            <LoadingRow label={t("common.loading")} />
          ) : !file ? (
            <EmptyState
              icon={<FileCode2 size={22} aria-hidden="true" />}
              title={t("editor.open_file")}
              description={<p>{t("editor.select_file")}</p>}
            />
          ) : (
            <>
              <div className="section-heading">
                <h2 id="editor-file" className="mono">
                  {file.path}
                </h2>
                <div className="inline-status">
                  {truncated ? <Badge tone="warning">{t("editor.truncated_badge")}</Badge> : null}
                  {dirty ? <Badge tone="warning">{t("editor.unsaved")}</Badge> : null}
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t("editor.close_file")}
                    onClick={requestClose}
                  >
                    <X size={15} />
                  </button>
                </div>
              </div>
              {truncated ? (
                <Notice tone="warning" title={t("editor.truncated_warning")}>
                  <p>{t("editor.read_only_hint")}</p>
                </Notice>
              ) : null}
              <div className="editor-surface">
                <pre className="line-gutter" aria-hidden="true" ref={gutterRef}>
                  {Array.from({ length: lineCount }, (_, index) => index + 1).join("\n")}
                </pre>
                <textarea
                  ref={textareaRef}
                  className="code-editor mono"
                  aria-label={t("editor.content_label")}
                  spellCheck={false}
                  wrap="off"
                  value={draft}
                  readOnly={truncated}
                  onChange={(event) => setDraft(event.target.value)}
                  onKeyDown={handleEditorKeyDown}
                  onScroll={() => {
                    if (gutterRef.current && textareaRef.current) {
                      gutterRef.current.scrollTop = textareaRef.current.scrollTop;
                    }
                  }}
                />
              </div>
              <p className="muted small">
                {t("editor.save_shortcut")}
                {dirty ? ` · ${t("editor.unsaved_indicator")}` : ""}
              </p>
            </>
          )}
        </section>
      </div>

      <ConfirmDialog
        open={pendingChange !== null}
        title={t("editor.unsaved")}
        description={<p>{t("editor.unsaved_switch")}</p>}
        confirmLabel={t("editor.discard")}
        onCancel={() => setPendingChange(null)}
        onConfirm={() => void confirmDiscard()}
        danger
      />
    </>
  );
}
