import { useEffect, useRef, useState } from "react";
import { Download, MessageSquare, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api, toAppError, type AppError, type Message, type Thread } from "../lib/api";
import { downloadText, safeFilename } from "../lib/download";
import { formatDate } from "../lib/format";
import { useAsync, useAction } from "../lib/hooks";
import { useApp } from "../lib/store";
import { Badge } from "../components/Badge";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { JsonView } from "../components/JsonView";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";

export function HistoryPage() {
  const { t, locale, project, config } = useApp();
  const threads = useAsync<Thread[]>(async () => {
    if (!project) {
      return [];
    }
    return api.history.threads(project.id);
  }, [project?.id]);
  const createAction = useAction();
  const deleteAction = useAction();
  const exportAction = useAction();
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [messagesError, setMessagesError] = useState<AppError | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [title, setTitle] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [toDelete, setToDelete] = useState<Thread | null>(null);
  const loadToken = useRef(0);

  const pageSize = Math.max(1, Math.min(500, config?.resources.transcript_page_size ?? 50));

  useEffect(() => {
    setSelectedThreadId(null);
    setMessages([]);
    setHasMore(false);
    setMessagesError(null);
    setTitle("");
  }, [project?.id]);

  useEffect(() => {
    if (threads.loading || !project) {
      return;
    }
    const available = (threads.data ?? []).filter((entry) => entry.project_id === project.id);
    if (selectedThreadId && available.some((entry) => entry.id === selectedThreadId)) {
      return;
    }
    setSelectedThreadId(available[0]?.id ?? null);
  }, [threads.data, threads.loading, selectedThreadId, project]);

  useEffect(() => {
    if (!selectedThreadId) {
      setMessages([]);
      setHasMore(false);
      return undefined;
    }
    const token = loadToken.current + 1;
    loadToken.current = token;
    setMessagesLoading(true);
    setMessagesError(null);
    api.history
      .messages(selectedThreadId, pageSize, 0)
      .then(
        (page) => {
          if (loadToken.current !== token) {
            return;
          }
          setMessages(page);
          setHasMore(page.length === pageSize);
        },
        (reason: unknown) => {
          if (loadToken.current === token) {
            setMessagesError(toAppError(reason));
          }
        },
      )
      .finally(() => {
        if (loadToken.current === token) {
          setMessagesLoading(false);
        }
      });
    return () => {
      loadToken.current += 1;
    };
  }, [selectedThreadId, pageSize]);

  const loadMore = async () => {
    if (!selectedThreadId || loadingMore) {
      return;
    }
    setLoadingMore(true);
    setMessagesError(null);
    const token = loadToken.current;
    try {
      const page = await api.history.messages(selectedThreadId, pageSize, messages.length);
      if (loadToken.current !== token) {
        return;
      }
      setMessages((previous) => [...previous, ...page]);
      setHasMore(page.length === pageSize);
    } catch (reason) {
      if (loadToken.current === token) {
        setMessagesError(toAppError(reason));
      }
    } finally {
      setLoadingMore(false);
    }
  };

  const createThread = async () => {
    if (!project || title.trim().length === 0) {
      return;
    }
    setMessage(null);
    const created = await createAction.run(() =>
      api.history.createThread(project.id, title.trim()),
    );
    if (created) {
      setTitle("");
      setMessage(t("history.created"));
      threads.reload();
      setSelectedThreadId(created.id);
    }
  };

  const deleteThread = async () => {
    if (!toDelete) {
      return;
    }
    const target = toDelete;
    const done = await deleteAction.run(async () => {
      await api.history.deleteThread(target.id);
      return true;
    });
    if (done) {
      setToDelete(null);
      setMessage(t("history.deleted"));
      if (selectedThreadId === target.id) {
        setSelectedThreadId(null);
      }
      threads.setData((current) => (current ?? []).filter((entry) => entry.id !== target.id));
      threads.reload();
    }
  };

  const exportHistory = async () => {
    if (!project) {
      return;
    }
    setMessage(null);
    const json = await exportAction.run(() => api.history.exportProject(project.id));
    if (json) {
      downloadText(`${safeFilename(project.name)}-history.json`, json);
      setMessage(t("history.exported"));
    }
  };

  const roleLabel = (role: string): string => {
    switch (role) {
      case "user":
        return t("history.role_user");
      case "assistant":
        return t("history.role_assistant");
      case "system":
        return t("history.role_system");
      case "tool":
        return t("history.role_tool");
      default:
        return role;
    }
  };

  if (!project) {
    return (
      <>
        <PageHeader title={t("history.title")} />
        <ProjectGate title={t("history.title")} />
      </>
    );
  }

  return (
    <>
      <PageHeader
        title={t("history.title")}
        subtitle={t("history.export_hint")}
        actions={
          <>
            <button type="button" className="ghost" onClick={() => threads.reload()}>
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={() => void exportHistory()}
              disabled={exportAction.busy}
              aria-busy={exportAction.busy}
            >
              {exportAction.busy ? <Spinner /> : <Download size={15} aria-hidden="true" />}
              {t("history.export")}
            </button>
          </>
        }
      />

      {threads.error ? <ErrorBanner error={threads.error} onRetry={threads.reload} /> : null}
      {createAction.error ? <ErrorBanner error={createAction.error} /> : null}
      {deleteAction.error ? <ErrorBanner error={deleteAction.error} /> : null}
      {exportAction.error ? <ErrorBanner error={exportAction.error} /> : null}
      {message ? <Notice tone="success">{message}</Notice> : null}

      <div className="history-layout">
        <section className="card history-threads" aria-labelledby="history-threads">
          <div className="section-heading">
            <h2 id="history-threads">{t("history.threads")}</h2>
          </div>
          <div className="input-row">
            <input
              className="input"
              value={title}
              placeholder={t("history.thread_title_placeholder")}
              aria-label={t("history.thread_title")}
              onChange={(event) => setTitle(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void createThread();
                }
              }}
            />
            <button
              type="button"
              className="primary small"
              onClick={() => void createThread()}
              disabled={createAction.busy || title.trim().length === 0}
              aria-busy={createAction.busy}
            >
              {createAction.busy ? <Spinner /> : <Plus size={13} aria-hidden="true" />}
              {t("common.create")}
            </button>
          </div>
          {threads.loading ? (
            <LoadingRow label={t("common.loading")} />
          ) : (threads.data ?? []).length === 0 ? (
            <EmptyState
              icon={<MessageSquare size={22} aria-hidden="true" />}
              title={t("history.no_threads")}
              description={<p>{t("history.no_threads_hint")}</p>}
            />
          ) : (
            <ul className="thread-list">
              {(threads.data ?? []).map((thread) => (
                <li
                  key={thread.id}
                  className={thread.id === selectedThreadId ? "thread-row selected" : "thread-row"}
                >
                  <button
                    type="button"
                    className="thread-button"
                    onClick={() => setSelectedThreadId(thread.id)}
                    aria-current={thread.id === selectedThreadId ? "true" : undefined}
                  >
                    <span className="thread-title">{thread.title}</span>
                    <span className="small muted">
                      {t("history.thread_created_at")} {formatDate(thread.created_at, locale)}
                    </span>
                  </button>
                  <button
                    type="button"
                    className="icon-button danger-text"
                    aria-label={t("history.delete_thread")}
                    onClick={() => {
                      deleteAction.reset();
                      setToDelete(thread);
                    }}
                  >
                    <Trash2 size={14} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="card history-messages" aria-labelledby="history-messages">
          <div className="section-heading">
            <h2 id="history-messages">{t("history.messages")}</h2>
            <span className="small muted">
              {t("history.page_size", { count: pageSize })}
            </span>
          </div>
          {messagesError ? <ErrorBanner error={messagesError} /> : null}
          {!selectedThreadId ? (
            <EmptyState title={t("history.select_thread")} />
          ) : messagesLoading ? (
            <LoadingRow label={t("common.loading")} />
          ) : messages.length === 0 ? (
            <EmptyState title={t("history.no_messages")} />
          ) : (
            <>
              <p className="small muted">
                {t("history.messages_loaded", { count: messages.length })}
              </p>
              <ul className="message-list">
                {messages.map((entry) => (
                  <li key={entry.id} className={`message message-${entry.role}`}>
                    <div className="message-head">
                      <Badge tone={entry.role === "user" ? "accent" : "neutral"}>
                        {roleLabel(entry.role)}
                      </Badge>
                      <span className="small muted">{formatDate(entry.created_at, locale)}</span>
                    </div>
                    <div className="message-content prewrap">{entry.content}</div>
                    {entry.metadata ? (
                      <details className="details">
                        <summary>{t("history.message_meta")}</summary>
                        <JsonView value={entry.metadata} maxHeight={220} />
                      </details>
                    ) : null}
                  </li>
                ))}
              </ul>
              {hasMore ? (
                <button
                  type="button"
                  className="ghost"
                  onClick={() => void loadMore()}
                  disabled={loadingMore}
                  aria-busy={loadingMore}
                >
                  {loadingMore ? <Spinner /> : null}
                  {t("history.load_more")}
                </button>
              ) : (
                <p className="muted small">{t("history.all_loaded")}</p>
              )}
            </>
          )}
        </section>
      </div>

      <ConfirmDialog
        open={toDelete !== null}
        title={t("history.delete_confirm_title", { title: toDelete?.title ?? "" })}
        description={<p>{t("history.delete_confirm_text")}</p>}
        confirmLabel={t("common.delete")}
        onCancel={() => setToDelete(null)}
        onConfirm={() => void deleteThread()}
        busy={deleteAction.busy}
        danger
      />
    </>
  );
}
