import { useState } from "react";
import { Brain, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { api, toAppError, type AppError, type MemoryEntry } from "../lib/api";
import { formatDate } from "../lib/format";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { Tabs, tabPanelId } from "../components/Tabs";

type MemoryScope = "project" | "global";

export function MemoryPage() {
  const { t, locale, project, navigate } = useApp();
  const [scope, setScope] = useState<MemoryScope>("project");
  const scopeProjectId = scope === "project" ? project?.id ?? null : null;
  const scopeReady = scope === "global" || project !== null;

  const entries = useAsync<MemoryEntry[]>(async () => {
    if (!scopeReady) {
      return [];
    }
    return api.memory.list(scopeProjectId);
  }, [scopeReady, scopeProjectId]);
  const preview = useAsync<string>(async () => {
    if (!scopeReady) {
      return "";
    }
    return api.memory.preview(scopeProjectId);
  }, [scopeReady, scopeProjectId]);
  const addAction = useAction();
  const removeAction = useAction();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<MemoryEntry[] | null>(null);
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchError, setSearchError] = useState<AppError | null>(null);
  const [form, setForm] = useState({ content: "", tags: "" });
  const [message, setMessage] = useState<string | null>(null);
  const [toDelete, setToDelete] = useState<MemoryEntry | null>(null);

  const visible = results ?? entries.data ?? [];

  const changeScope = (next: MemoryScope) => {
    setScope(next);
    setResults(null);
    setQuery("");
    setSearchError(null);
    setMessage(null);
  };

  const addEntry = async () => {
    if (!scopeReady || form.content.trim().length === 0) {
      return;
    }
    setMessage(null);
    const tags = form.tags
      .split(",")
      .map((tag) => tag.trim())
      .filter((tag) => tag.length > 0);
    const added = await addAction.run(() =>
      api.memory.add(scopeProjectId, form.content.trim(), tags),
    );
    if (added) {
      setForm({ content: "", tags: "" });
      setMessage(t("memory.added"));
      entries.reload();
      preview.reload();
    }
  };

  const runSearch = async () => {
    if (!scopeReady || query.trim().length === 0) {
      return;
    }
    setSearchBusy(true);
    setSearchError(null);
    try {
      setResults(await api.memory.search(scopeProjectId, query.trim()));
    } catch (reason) {
      setSearchError(toAppError(reason));
      setResults(null);
    } finally {
      setSearchBusy(false);
    }
  };

  const deleteEntry = async () => {
    if (!toDelete) {
      return;
    }
    const target = toDelete;
    const done = await removeAction.run(async () => {
      await api.memory.remove(target.id);
      return true;
    });
    if (done) {
      setToDelete(null);
      setMessage(t("memory.deleted"));
      entries.reload();
      preview.reload();
      if (results !== null) {
        setResults(results.filter((entry) => entry.id !== target.id));
      }
    }
  };

  return (
    <>
      <PageHeader
        title={t("memory.title")}
        subtitle={scope === "project" ? t("memory.scope_hint") : t("memory.global_hint")}
        actions={
          <button type="button" className="ghost" onClick={() => void entries.reload()}>
            <RefreshCw size={15} aria-hidden="true" />
            {t("common.refresh")}
          </button>
        }
      />

      <Tabs
        idPrefix="memory"
        label={t("memory.scope")}
        active={scope}
        onChange={(next) => changeScope(next)}
        tabs={[
          { id: "project", label: t("memory.project") },
          { id: "global", label: t("memory.global") },
        ]}
      />

      <div role="tabpanel" id={tabPanelId("memory", scope)} aria-labelledby={`memory-tab-${scope}`}>
      {!scopeReady ? (
        <Notice tone="warning" title={t("gate.title")}>
          <p>{t("gate.text")}</p>
          <button type="button" className="ghost small" onClick={() => navigate("projects")}>
            {t("gate.action")}
          </button>
        </Notice>
      ) : null}

      {entries.error ? <ErrorBanner error={entries.error} onRetry={entries.reload} /> : null}
      {addAction.error ? <ErrorBanner error={addAction.error} /> : null}
      {removeAction.error ? <ErrorBanner error={removeAction.error} /> : null}
      {searchError ? <ErrorBanner error={searchError} /> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}

      <div className="memory-layout">
        <section className="card" aria-labelledby="memory-add">
          <div className="section-heading">
            <h2 id="memory-add">{t("memory.add_entry")}</h2>
            <Plus size={17} aria-hidden="true" className="muted-icon" />
          </div>
          <Field label={t("memory.content")} htmlFor="memory-content" required>
            <textarea
              id="memory-content"
              className="input textarea"
              rows={5}
              value={form.content}
              placeholder={t("memory.content_placeholder")}
              onChange={(event) => setForm({ ...form, content: event.target.value })}
            />
          </Field>
          <Field label={t("memory.tags")} htmlFor="memory-tags" hint={t("memory.tags_hint")}>
            <input
              id="memory-tags"
              className="input"
              value={form.tags}
              onChange={(event) => setForm({ ...form, tags: event.target.value })}
            />
          </Field>
          <button
            type="button"
            className="primary"
            onClick={() => void addEntry()}
            disabled={!scopeReady || addAction.busy || form.content.trim().length === 0}
            aria-busy={addAction.busy}
          >
            {addAction.busy ? <Spinner /> : <Plus size={14} aria-hidden="true" />}
            {t("memory.add")}
          </button>
        </section>

        <section className="card" aria-labelledby="memory-preview">
          <div className="section-heading">
            <h2 id="memory-preview">{t("memory.preview")}</h2>
            <button
              type="button"
              className="ghost small"
              onClick={() => preview.reload()}
              disabled={!scopeReady}
            >
              <RefreshCw size={13} aria-hidden="true" />
              {t("common.refresh")}
            </button>
          </div>
          <p className="muted small">{t("memory.preview_hint")}</p>
          {preview.error ? <ErrorBanner error={preview.error} /> : null}
          {preview.loading ? (
            <LoadingRow label={t("common.loading")} />
          ) : (
            <pre className="code-block memory-preview">
              {preview.data && preview.data.length > 0
                ? preview.data
                : t("memory.preview_empty")}
            </pre>
          )}
        </section>
      </div>

      <section className="card" aria-labelledby="memory-entries">
        <div className="section-heading">
          <h2 id="memory-entries">
            {results !== null ? t("memory.search_results") : t("memory.entries")}
          </h2>
          <div className="inline-status">
            <Badge tone="neutral">{t("memory.results", { count: visible.length })}</Badge>
            {results !== null ? (
              <button
                type="button"
                className="ghost small"
                onClick={() => {
                  setResults(null);
                  setQuery("");
                }}
              >
                <X size={13} aria-hidden="true" />
                {t("memory.clear_search")}
              </button>
            ) : null}
          </div>
        </div>

        <div className="input-row memory-search">
          <input
            className="input"
            value={query}
            placeholder={t("memory.search_placeholder")}
            aria-label={t("memory.search")}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void runSearch();
              }
            }}
            disabled={!scopeReady}
          />
          <button
            type="button"
            className="ghost small"
            onClick={() => void runSearch()}
            disabled={!scopeReady || searchBusy || query.trim().length === 0}
            aria-busy={searchBusy}
          >
            {searchBusy ? <Spinner /> : <Search size={13} aria-hidden="true" />}
            {t("memory.search")}
          </button>
        </div>

        {entries.loading && results === null ? (
          <LoadingRow label={t("common.loading")} />
        ) : visible.length === 0 ? (
          <EmptyState
            icon={<Brain size={22} aria-hidden="true" />}
            title={t("memory.no_entries")}
            description={<p>{t("memory.empty_hint")}</p>}
          />
        ) : (
          <ul className="memory-list">
            {visible.map((entry) => (
              <li key={entry.id} className="memory-entry">
                <div className="memory-entry-head">
                  <Badge tone={entry.scope === "global" ? "info" : "accent"}>
                    {entry.scope === "global" ? t("memory.global") : t("memory.project")}
                  </Badge>
                  <span className="small muted">
                    {t("memory.source")} : {entry.source} · {formatDate(entry.created_at, locale)}
                  </span>
                  <button
                    type="button"
                    className="icon-button danger-text"
                    aria-label={t("memory.delete_entry")}
                    onClick={() => {
                      removeAction.reset();
                      setToDelete(entry);
                    }}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
                <p className="prewrap">{entry.content}</p>
                {entry.tags.length > 0 ? (
                  <div className="tag-list">
                    {entry.tags.map((tag, index) => (
                      <Badge key={`${tag}-${index}`} tone="neutral">
                        {tag}
                      </Badge>
                    ))}
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </section>

      </div>

      <ConfirmDialog
        open={toDelete !== null}
        title={t("memory.delete_confirm_title")}
        description={<p>{t("memory.delete_confirm_text")}</p>}
        confirmLabel={t("common.delete")}
        onCancel={() => setToDelete(null)}
        onConfirm={() => void deleteEntry()}
        busy={removeAction.busy}
        danger
      />
    </>
  );
}
