import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileCode2,
  Folder,
  FolderOpen,
  PanelLeftClose,
  RefreshCw,
  Search,
  X,
} from "lucide-react";
import { api, toAppError, type AppError, type FileMatch } from "../lib/api";
import { buildFileTree, type TreeNode } from "../lib/fs";
import { useAsync, useDebouncedValue } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "./Badge";
import { EmptyState, LoadingRow, Spinner } from "./EmptyState";
import { ErrorBanner } from "./ErrorBanner";

/**
 * Explorateur de fichiers du workspace : arborescence repliable (dossiers
 * d'abord) et recherche par nom via `files_match`. La recherche remplace
 * l'arbre ; les fichiers exclus du contexte sont signalés.
 */
export function FileExplorer({
  projectId,
  projectName,
  activePath,
  width,
  onOpen,
  onHide,
}: {
  projectId: string;
  projectName: string;
  activePath: string | null;
  width: number;
  onOpen: (path: string) => void;
  onHide: () => void;
}) {
  const { t } = useApp();
  const tree = useAsync(() => api.files.tree(projectId, 6), [projectId]);
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, 150);
  const [matches, setMatches] = useState<FileMatch[] | null>(null);
  const [matchLoading, setMatchLoading] = useState(false);
  const [matchError, setMatchError] = useState<AppError | null>(null);

  const nodes = useMemo(() => buildFileTree(tree.data ?? []), [tree.data]);
  const searching = debounced.trim().length > 0;

  useEffect(() => {
    const needle = debounced.trim();
    if (needle.length === 0) {
      setMatches(null);
      setMatchError(null);
      setMatchLoading(false);
      return undefined;
    }
    let cancelled = false;
    setMatchLoading(true);
    setMatchError(null);
    api.workspace.match(projectId, needle, 40).then(
      (results) => {
        if (!cancelled) {
          setMatches(results);
          setMatchLoading(false);
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setMatchError(toAppError(reason));
          setMatchLoading(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [debounced, projectId]);

  const renderNodes = (entries: TreeNode[], depth: number): ReactNode =>
    entries.map((entry) =>
      entry.node.is_dir ? (
        <details key={entry.node.path} className="tree-folder" open={depth < 1}>
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
          className={activePath === entry.node.path ? "tree-file active" : "tree-file"}
          onClick={() => onOpen(entry.node.path)}
          title={entry.node.path}
        >
          <FileCode2 size={13} aria-hidden="true" />
          <span>{entry.node.name}</span>
        </button>
      ),
    );

  return (
    <aside
      className="card workspace-explorer"
      style={{ width }}
      aria-labelledby="workspace-explorer-title"
    >
      <div className="workspace-explorer-head">
        <div className="workspace-explorer-title">
          <h2 id="workspace-explorer-title">{t("workspace.explorer")}</h2>
          <span className="small muted" title={projectName}>
            {projectName}
          </span>
        </div>
        <div className="button-row compact">
          <button
            type="button"
            className="icon-button"
            aria-label={t("workspace.refresh_tree")}
            title={t("workspace.refresh_tree")}
            onClick={() => tree.reload()}
            disabled={tree.loading}
          >
            {tree.loading ? <Spinner /> : <RefreshCw size={14} />}
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label={t("workspace.hide_explorer")}
            title={t("workspace.hide_explorer")}
            onClick={onHide}
          >
            <PanelLeftClose size={14} />
          </button>
        </div>
      </div>

      <div className="chat-search workspace-explorer-search">
        <Search size={14} aria-hidden="true" className="chat-search-icon" />
        <input
          className="input"
          type="search"
          value={query}
          placeholder={t("workspace.file_search_placeholder")}
          aria-label={t("workspace.file_search_placeholder")}
          onChange={(event) => setQuery(event.target.value)}
        />
        {query.length > 0 ? (
          <button
            type="button"
            className="icon-button chat-search-clear"
            aria-label={t("workspace.clear_search")}
            onClick={() => setQuery("")}
          >
            <X size={14} />
          </button>
        ) : null}
        {matchLoading ? <Spinner /> : null}
      </div>

      {tree.error ? <ErrorBanner error={tree.error} onRetry={tree.reload} /> : null}
      {matchError ? <ErrorBanner error={matchError} /> : null}

      <div className="workspace-explorer-body">
        {searching ? (
          matchLoading && matches === null ? (
            <LoadingRow label={t("common.loading")} />
          ) : matches === null || matches.length === 0 ? (
            <EmptyState
              icon={<Search size={20} aria-hidden="true" />}
              title={t("workspace.no_match")}
              description={
                <p>
                  {matchLoading
                    ? t("workspace.searching")
                    : t("workspace.no_match_hint", { query: debounced.trim() })}
                </p>
              }
            />
          ) : (
            <ul className="workspace-match-list" aria-label={t("workspace.search_results")}>
              {matches.map((match) => (
                <li key={`${match.path}-${match.is_dir ? "d" : "f"}`}>
                  {match.is_dir ? (
                    <div className="workspace-match-row muted" title={match.path}>
                      <FolderOpen size={13} aria-hidden="true" />
                      <span className="mono small">{match.path}</span>
                      <Badge tone="neutral">{t("workspace.folder")}</Badge>
                    </div>
                  ) : (
                    <button
                      type="button"
                      className={
                        activePath === match.path
                          ? "workspace-match-row active"
                          : "workspace-match-row"
                      }
                      onClick={() => onOpen(match.path)}
                      title={match.path}
                    >
                      <FileCode2 size={13} aria-hidden="true" />
                      <span className="mono small">{match.path}</span>
                      {match.excluded ? (
                        <Badge tone="warning">{t("workspace.excluded")}</Badge>
                      ) : null}
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )
        ) : tree.loading && tree.data === null ? (
          <LoadingRow label={t("workspace.tree_loading")} />
        ) : nodes.length === 0 ? (
          <EmptyState
            icon={<Folder size={20} aria-hidden="true" />}
            title={t("workspace.no_files")}
            description={<p>{t("workspace.no_files_hint")}</p>}
          />
        ) : (
          <div className="tree workspace-tree" aria-label={t("workspace.tree_label")}>
            {renderNodes(nodes, 0)}
          </div>
        )}
      </div>
    </aside>
  );
}
