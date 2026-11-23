import {
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  Activity,
  AtSign,
  Ban,
  Bot,
  FileCode2,
  MessageSquare,
  MessagesSquare,
  MoreHorizontal,
  PanelBottom,
  PanelLeft,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  Send,
  ShieldAlert,
  Trash2,
  X,
} from "lucide-react";
import {
  AppError,
  api,
  toAppError,
  type ChatUsage,
  type FileMatch,
  type Message,
  type ModelEntry,
  type PreparedContext,
  type ProviderView,
  type Thread,
} from "../lib/api";
import {
  INITIAL_CHAT_STREAM,
  DEFAULT_THREAD_TITLE,
  chatStreamReducer,
  displayThreadTitle,
  filterThreads,
  formatRelativeTime,
  messageFromOutcome,
  readMessageMetadata,
  sortThreadsByUpdated,
  totalTokens,
  type ChatDeltaEvent,
  type ChatDoneEvent,
  type ChatErrorEvent,
  type ChatStreamAction,
} from "../lib/chat";
import { agentPhaseLabelKey } from "../lib/agent";
import { errorIsFixable } from "../lib/errors";
import { useTauriEvent } from "../lib/events";
import { formatBytes, formatDate } from "../lib/format";
import {
  MIN_BOTTOM_HEIGHT,
  MIN_CONVERSATION_WIDTH,
  MIN_EDITOR_WIDTH,
  MIN_EXPLORER_WIDTH,
  maxBottomHeight,
  maxConversationWidth,
  maxExplorerWidth,
} from "../lib/layout";
import {
  addChip,
  clearChips,
  extractContextPaths,
  findMentionToken,
  insertMention,
  removeChip,
  type ContextChip,
} from "../lib/mentions";
import { useAgentSession } from "../lib/useAgentSession";
import { useWorkspace } from "../lib/useWorkspace";
import { useAction, useAsync, useDebouncedValue } from "../lib/hooks";
import { useApp } from "../lib/store";
import { AgentPanel } from "../components/AgentPanel";
import { Badge, type BadgeTone } from "../components/Badge";
import { BottomPanel } from "../components/BottomPanel";
import { ChatModeSwitch, type ChatMode } from "../components/ChatModeSwitch";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { EmptyState, LoadingRow, Spinner } from "../components/EmptyState";
import { ErrorBanner } from "../components/ErrorBanner";
import { Field } from "../components/Field";
import { FileExplorer } from "../components/FileExplorer";
import { Modal } from "../components/Modal";
import { Notice } from "../components/Notice";
import { PageHeader } from "../components/PageHeader";
import { ProjectGate } from "../components/ProjectGate";
import { Resizer } from "../components/Resizer";
import { WorkspaceEditor } from "../components/WorkspaceEditor";

const MESSAGE_LIMIT = 500;
const ACTIVE_POLL_MS = 5_000;

interface ActiveTurn {
  requestId: string;
  threadId: string;
  providerId: string;
  model: string;
  prompt: string;
  localUserId: string;
  startedAt: number;
  finished: boolean;
  contextPaths: string[];
  allowSensitiveContext: boolean;
}

interface RetryInfo {
  prompt: string;
  providerId: string;
  model: string;
  threadId: string | null;
  contextPaths: string[];
  allowSensitiveContext: boolean;
}

interface LastTurnStats {
  elapsedMs: number;
  usage: ChatUsage;
  cancelled: boolean;
  threadId: string;
}

interface NoticeState {
  text: string;
  tone: "info" | "success" | "warning";
}

type TerminalAction = Extract<ChatStreamAction, { type: "done" | "error" }>;

function roleLabel(t: (key: string) => string, role: string): string {
  switch (role) {
    case "user":
      return t("chat.role_user");
    case "assistant":
      return t("chat.role_assistant");
    case "system":
      return t("chat.role_system");
    case "tool":
      return t("chat.role_tool");
    default:
      return role;
  }
}

function roleTone(role: string): BadgeTone {
  switch (role) {
    case "user":
      return "accent";
    case "assistant":
      return "info";
    case "system":
      return "warning";
    default:
      return "neutral";
  }
}

export function ChatPage() {
  const { t, locale, project, navigate } = useApp();
  const ws = useWorkspace(project?.id ?? null);
  // Le mode Agent vit au-dessus du panneau : changer de mode ne perd pas la session.
  const [chatMode, setChatMode] = useState<ChatMode>("chat");
  const agentSession = useAgentSession(project?.id ?? null);
  const projectIdRef = useRef<string | null>(project?.id ?? null);
  projectIdRef.current = project?.id ?? null;

  // --------------------------------------------------------------- fournisseurs
  const providers = useAsync<ProviderView[]>(() => api.providers.list(), []);
  const enabledProviders = useMemo(
    () => (providers.data ?? []).filter((entry) => entry.enabled),
    [providers.data],
  );
  const disabledProviders = useMemo(
    () => (providers.data ?? []).filter((entry) => !entry.enabled),
    [providers.data],
  );
  const [providerId, setProviderId] = useState("");
  const [model, setModel] = useState("");
  const [discovered, setDiscovered] = useState<Record<string, ModelEntry[]>>({});
  const previousProviderRef = useRef<string | null>(null);
  const saveDefault = useAction();
  const discover = useAction();

  const selectedProvider = useMemo(
    () => enabledProviders.find((entry) => entry.id === providerId) ?? null,
    [enabledProviders, providerId],
  );

  const modelOptions = useMemo<ModelEntry[]>(() => {
    if (!selectedProvider) {
      return [];
    }
    const merged = [...selectedProvider.models];
    for (const entry of discovered[selectedProvider.id] ?? []) {
      if (!merged.some((candidate) => candidate.id === entry.id)) {
        merged.push(entry);
      }
    }
    if (
      selectedProvider.default_model &&
      !merged.some((candidate) => candidate.id === selectedProvider.default_model)
    ) {
      // Le modèle par défaut vient de la configuration réelle : il n'est pas inventé.
      merged.unshift({
        id: selectedProvider.default_model,
        label: selectedProvider.default_model,
        context_window: null,
        supports_tools: false,
        supports_vision: false,
        local: selectedProvider.kind === "ollama",
      });
    }
    return merged;
  }, [selectedProvider, discovered]);

  useEffect(() => {
    if (providers.loading || !providers.data) {
      return;
    }
    if (providerId && enabledProviders.some((entry) => entry.id === providerId)) {
      return;
    }
    const preferred =
      enabledProviders.find((entry) => entry.default_model) ?? enabledProviders[0] ?? null;
    setProviderId(preferred?.id ?? "");
  }, [providers.loading, providers.data, enabledProviders, providerId]);

  useEffect(() => {
    if (!selectedProvider) {
      previousProviderRef.current = null;
      setModel("");
      return;
    }
    const changed = previousProviderRef.current !== selectedProvider.id;
    previousProviderRef.current = selectedProvider.id;
    setModel((current) => {
      const known =
        modelOptions.some((entry) => entry.id === current) ||
        (selectedProvider.kind === "ollama" && current.trim().length > 0);
      if (!changed && known) {
        return current;
      }
      if (selectedProvider.default_model) {
        return selectedProvider.default_model;
      }
      return modelOptions[0]?.id ?? "";
    });
  }, [selectedProvider, modelOptions]);

  const persistDefaultModel = async (targetProviderId: string, value: string) => {
    const trimmed = value.trim();
    if (!trimmed) {
      return;
    }
    const saved = await saveDefault.run(async () => {
      await api.providers.setDefaultModel(targetProviderId, trimmed);
      return true;
    });
    if (saved) {
      providers.setData((current) =>
        current
          ? current.map((entry) =>
              entry.id === targetProviderId ? { ...entry, default_model: trimmed } : entry,
            )
          : current,
      );
    }
  };

  const handleModelChange = (value: string) => {
    setModel(value);
    if (selectedProvider && value.trim().length > 0) {
      void persistDefaultModel(selectedProvider.id, value);
    }
  };

  const discoverModels = async () => {
    if (!selectedProvider) {
      return;
    }
    const target = selectedProvider;
    const found = await discover.run(() => api.providers.models(target.id));
    if (!found) {
      return;
    }
    setDiscovered((current) => ({ ...current, [target.id]: found }));
    setNotice({
      text:
        found.length === 0
          ? t("chat.discover_empty")
          : t("chat.discovered", { count: found.length }),
      tone: found.length === 0 ? "warning" : "success",
    });
  };

  // ------------------------------------------------------------------- fils
  const [search, setSearch] = useState("");
  const debouncedSearch = useDebouncedValue(search, 250);
  const threads = useAsync<Thread[]>(async () => {
    if (!project) {
      return [];
    }
    const query = debouncedSearch.trim();
    const raw = query
      ? await api.history.search(project.id, query)
      : await api.history.threads(project.id);
    return sortThreadsByUpdated(filterThreads(raw, project.id));
  }, [project?.id, debouncedSearch]);

  const [selectedThreadId, setSelectedThreadIdState] = useState<string | null>(null);
  const [selectedThreadSnapshot, setSelectedThreadSnapshot] = useState<Thread | null>(null);
  const selectedThreadRef = useRef<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [messagesError, setMessagesError] = useState<AppError | null>(null);
  const messagesToken = useRef(0);
  const messageListRef = useRef<HTMLUListElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const selectedThread =
    (threads.data ?? []).find((entry) => entry.id === selectedThreadId) ??
    (selectedThreadSnapshot?.id === selectedThreadId ? selectedThreadSnapshot : null);

  const selectThread = (threadId: string | null, thread: Thread | null = null) => {
    if (selectedThreadRef.current !== threadId) {
      messagesToken.current += 1;
      setMessages([]);
      setMessagesError(null);
      setMessagesLoading(false);
    }
    selectedThreadRef.current = threadId;
    setSelectedThreadIdState(threadId);
    setSelectedThreadSnapshot(thread);
  };

  const createThreadAction = useAction();
  const renameAction = useAction();
  const deleteAction = useAction();
  const [renameTarget, setRenameTarget] = useState<Thread | null>(null);
  const [renameTitle, setRenameTitle] = useState("");
  const [toDelete, setToDelete] = useState<Thread | null>(null);
  const [notice, setNotice] = useState<NoticeState | null>(null);

  // ----------------------------------------------------------------- messages
  const loadMessages = async (threadId: string): Promise<boolean> => {
    const token = messagesToken.current + 1;
    messagesToken.current = token;
    setMessagesLoading(true);
    setMessagesError(null);
    try {
      const page = await api.history.messages(threadId, MESSAGE_LIMIT, 0);
      if (messagesToken.current !== token || selectedThreadRef.current !== threadId) {
        return false;
      }
      setMessages(page);
      return true;
    } catch (reason) {
      if (messagesToken.current === token && selectedThreadRef.current === threadId) {
        setMessagesError(toAppError(reason));
      }
      return false;
    } finally {
      if (messagesToken.current === token) {
        setMessagesLoading(false);
      }
    }
  };

  useEffect(() => {
    if (!selectedThreadId) {
      messagesToken.current += 1;
      setMessages([]);
      setMessagesLoading(false);
      setMessagesError(null);
      return;
    }
    void loadMessages(selectedThreadId);
    // `loadMessages` est recréé à chaque rendu ; seule la sélection compte ici.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedThreadId]);

  useEffect(() => {
    if (!selectedThreadId) {
      setSelectedThreadSnapshot(null);
      return;
    }
    const fresh = (threads.data ?? []).find((entry) => entry.id === selectedThreadId);
    if (fresh) {
      setSelectedThreadSnapshot(fresh);
    }
  }, [threads.data, selectedThreadId]);

  useEffect(() => {
    if (!project || threads.loading) {
      return;
    }
    const list = threads.data ?? [];
    if (selectedThreadId && list.some((entry) => entry.id === selectedThreadId)) {
      return;
    }
    // Une recherche peut masquer la conversation courante : on la conserve.
    if (selectedThreadId && debouncedSearch.trim().length > 0) {
      return;
    }
    const first = list[0] ?? null;
    selectThread(first?.id ?? null, first);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project, threads.loading, threads.data, selectedThreadId, debouncedSearch]);

  // --------------------------------------------------------------- streaming
  const [stream, dispatchStream] = useReducer(chatStreamReducer, INITIAL_CHAT_STREAM);
  const streamRef = useRef(stream);
  streamRef.current = stream;
  const [activeTurn, setActiveTurnState] = useState<ActiveTurn | null>(null);
  const activeTurnRef = useRef<ActiveTurn | null>(null);
  const [streamError, setStreamError] = useState<AppError | null>(null);
  const [retryInfo, setRetryInfo] = useState<RetryInfo | null>(null);
  const [stopping, setStopping] = useState(false);
  const [lastTurn, setLastTurn] = useState<LastTurnStats | null>(null);
  const [activeCount, setActiveCount] = useState<number | null>(null);
  const sendPendingRef = useRef(false);
  const finalizingRef = useRef<string | null>(null);
  const pendingDeltasRef = useRef(new Map<string, string>());
  const terminalRef = useRef(new Map<string, TerminalAction>());

  const setActiveTurn = (turn: ActiveTurn | null) => {
    activeTurnRef.current = turn;
    setActiveTurnState(turn);
  };

  const refreshActive = async () => {
    try {
      setActiveCount(await api.chat.active());
    } catch {
      setActiveCount(null);
    }
  };

  /**
   * Libère l'entrée du registre côté noyau après la fin d'un tour. Le noyau
   * renvoie `false` si elle est déjà retirée ; l'appel reste donc sans effet.
   */
  const releaseRequest = async (requestId: string): Promise<void> => {
    try {
      await api.chat.cancel(requestId);
    } catch {
      // Le tour est terminé : l'échec de nettoyage n'a aucune conséquence.
    }
  };

  useEffect(() => {
    void refreshActive();
    const timer = window.setInterval(() => void refreshActive(), ACTIVE_POLL_MS);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // --------------------------------------------------------- fin de tour
  const finishDone = async (payload: ChatDoneEvent, turn: ActiveTurn) => {
    dispatchStream({
      type: "done",
      requestId: payload.request_id,
      threadId: payload.thread_id,
      outcome: payload.outcome,
    });
    setLastTurn({
      elapsedMs: payload.outcome.elapsed_ms,
      usage: payload.outcome.usage,
      cancelled: payload.outcome.cancelled,
      threadId: payload.thread_id,
    });
    setStreamError(null);
    setRetryInfo(null);
    setStopping(false);
    setActiveTurn({ ...turn, finished: true });
    await releaseRequest(payload.request_id);
    const wasSelected = selectedThreadRef.current === payload.thread_id;
    const loaded = wasSelected ? await loadMessages(payload.thread_id) : true;
    if (streamRef.current.requestId !== payload.request_id) {
      // Un autre tour (ou une réinitialisation) a pris la main : ne pas l'écraser.
      finalizingRef.current = null;
      return;
    }
    if (loaded) {
      dispatchStream({ type: "reset" });
    }
    if (
      activeTurnRef.current === null ||
      activeTurnRef.current.requestId === payload.request_id
    ) {
      setActiveTurn(null);
    }
    finalizingRef.current = null;
    threads.reload();
    void refreshActive();
  };

  const finishError = async (payload: ChatErrorEvent, turn: ActiveTurn) => {
    dispatchStream({
      type: "error",
      requestId: payload.request_id,
      threadId: payload.thread_id,
      code: payload.code,
      message: payload.message,
    });
    setStreamError(new AppError(payload.code, payload.message, errorIsFixable(payload.code)));
    setRetryInfo({
      prompt: turn.prompt,
      providerId: turn.providerId,
      model: turn.model,
      threadId: payload.thread_id,
      contextPaths: turn.contextPaths,
      allowSensitiveContext: turn.allowSensitiveContext,
    });
    setLastTurn(null);
    setStopping(false);
    setActiveTurn(null);
    await releaseRequest(payload.request_id);
    if (selectedThreadRef.current === payload.thread_id) {
      await loadMessages(payload.thread_id);
    }
    if (streamRef.current.requestId !== payload.request_id) {
      finalizingRef.current = null;
      return;
    }
    dispatchStream({ type: "reset" });
    if (
      activeTurnRef.current === null ||
      activeTurnRef.current.requestId === payload.request_id
    ) {
      setActiveTurn(null);
    }
    finalizingRef.current = null;
    threads.reload();
    void refreshActive();
  };

  const handleDelta = (payload: ChatDeltaEvent) => {
    const turn = activeTurnRef.current;
    if (turn && turn.requestId === payload.request_id) {
      dispatchStream({
        type: "delta",
        requestId: payload.request_id,
        threadId: payload.thread_id,
        delta: payload.delta,
      });
      return;
    }
    if (!turn && sendPendingRef.current) {
      const previous = pendingDeltasRef.current.get(payload.request_id) ?? "";
      pendingDeltasRef.current.set(payload.request_id, previous + payload.delta);
    }
  };

  const handleDone = (payload: ChatDoneEvent) => {
    const turn = activeTurnRef.current;
    if (turn && turn.requestId === payload.request_id) {
      if (finalizingRef.current === payload.request_id) {
        return;
      }
      finalizingRef.current = payload.request_id;
      void finishDone(payload, turn);
      return;
    }
    if (!turn && sendPendingRef.current) {
      terminalRef.current.set(payload.request_id, {
        type: "done",
        requestId: payload.request_id,
        threadId: payload.thread_id,
        outcome: payload.outcome,
      });
    }
  };

  const handleError = (payload: ChatErrorEvent) => {
    const turn = activeTurnRef.current;
    if (turn && turn.requestId === payload.request_id) {
      if (finalizingRef.current === payload.request_id) {
        return;
      }
      finalizingRef.current = payload.request_id;
      void finishError(payload, turn);
      return;
    }
    if (!turn && sendPendingRef.current) {
      terminalRef.current.set(payload.request_id, {
        type: "error",
        requestId: payload.request_id,
        threadId: payload.thread_id,
        code: payload.code,
        message: payload.message,
      });
    }
  };

  const deltaListenerError = useTauriEvent<ChatDeltaEvent>("ih://chat-delta", handleDelta);
  const doneListenerError = useTauriEvent<ChatDoneEvent>("ih://chat-done", handleDone);
  const errorListenerError = useTauriEvent<ChatErrorEvent>("ih://chat-error", handleError);

  // --------------------------------------------------------------- envoi
  const sendAction = useAction();
  const cancelAction = useAction();
  const [prompt, setPrompt] = useState("");

  // -------------------------------------------------- contexte et mentions @
  const [chips, setChips] = useState<ContextChip[]>([]);
  const [allowSensitiveContext, setAllowSensitiveContext] = useState(false);
  const [sensitiveConfirm, setSensitiveConfirm] = useState(false);
  const [mentionResults, setMentionResults] = useState<FileMatch[]>([]);
  const [mentionLoading, setMentionLoading] = useState(false);
  const [mentionError, setMentionError] = useState<AppError | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionDismissed, setMentionDismissed] = useState(false);
  const [prepared, setPrepared] = useState<PreparedContext | null>(null);
  const [prepareError, setPrepareError] = useState<AppError | null>(null);
  const [prepareBusy, setPrepareBusy] = useState(false);
  const [prepareNonce, setPrepareNonce] = useState(0);

  const mentionToken = useMemo(() => findMentionToken(prompt), [prompt]);
  const mentionOpen = mentionToken !== null && !mentionDismissed;
  const debouncedMentionQuery = useDebouncedValue(
    mentionOpen && mentionToken ? mentionToken.query : null,
    150,
  );

  useEffect(() => {
    if (!project || !mentionOpen || debouncedMentionQuery === null) {
      setMentionResults([]);
      setMentionLoading(false);
      setMentionError(null);
      return undefined;
    }
    let cancelled = false;
    setMentionLoading(true);
    setMentionError(null);
    api.workspace.match(project.id, debouncedMentionQuery, 20).then(
      (results) => {
        if (!cancelled) {
          setMentionResults(results.filter((entry) => !entry.is_dir));
          setMentionIndex(0);
          setMentionLoading(false);
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setMentionError(toAppError(reason));
          setMentionLoading(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [project?.id, mentionOpen, debouncedMentionQuery]);

  const chooseMention = (match: FileMatch) => {
    if (!project || !mentionToken) {
      return;
    }
    const scope = project.id;
    const inserted = insertMention(prompt, mentionToken, match.path);
    setPrompt(inserted.text);
    setMentionDismissed(true);
    setMentionResults([]);
    setChips((current) => addChip(current, { path: match.path, size: null }));
    composerRef.current?.focus();
    window.requestAnimationFrame(() => {
      const element = composerRef.current;
      if (element) {
        element.setSelectionRange(inserted.caret, inserted.caret);
      }
    });
    void api.workspace.meta(scope, match.path).then(
      (meta) => {
        if (projectIdRef.current === scope) {
          setChips((current) => addChip(current, { path: match.path, size: meta.size }));
        }
      },
      () => undefined,
    );
  };

  const handleMentionKeyDown = (event: ReactKeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!mentionOpen) {
      return false;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setMentionIndex((current) =>
        mentionResults.length === 0 ? 0 : (current + 1) % mentionResults.length,
      );
      return true;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setMentionIndex((current) =>
        mentionResults.length === 0
          ? 0
          : (current - 1 + mentionResults.length) % mentionResults.length,
      );
      return true;
    }
    if (event.key === "Enter") {
      const target = mentionResults[mentionIndex];
      if (target) {
        event.preventDefault();
        chooseMention(target);
        return true;
      }
      return false;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setMentionDismissed(true);
      return true;
    }
    return false;
  };

  const chipPaths = useMemo(() => extractContextPaths(chips), [chips]);
  const chipKey = chipPaths.join("\n");
  const debouncedChipKey = useDebouncedValue(chipKey, 300);

  useEffect(() => {
    if (!project) {
      setPrepared(null);
      setPrepareError(null);
      setPrepareBusy(false);
      return undefined;
    }
    const paths = debouncedChipKey.length > 0 ? debouncedChipKey.split("\n") : [];
    if (paths.length === 0) {
      setPrepared(null);
      setPrepareError(null);
      setPrepareBusy(false);
      return undefined;
    }
    let cancelled = false;
    setPrepareBusy(true);
    setPrepareError(null);
    api.workspace.prepareContext(project.id, paths, allowSensitiveContext).then(
      (result) => {
        if (!cancelled) {
          setPrepared(result);
          setPrepareBusy(false);
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setPrepareError(toAppError(reason));
          setPrepareBusy(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [project?.id, debouncedChipKey, allowSensitiveContext, prepareNonce]);

  const sensitiveSkipped = useMemo(
    () => (prepared?.skipped ?? []).filter((entry) => /sensible|sensitive/i.test(entry.reason)),
    [prepared],
  );

  const clearContext = () => {
    setChips(clearChips());
    setAllowSensitiveContext(false);
    setSensitiveConfirm(false);
    setPrepared(null);
    setPrepareError(null);
  };

  const removeContextChip = (path: string) => {
    setChips((current) => removeChip(current, path));
    // Retirer un fichier ne doit pas laisser une autorisation sensible active.
    setAllowSensitiveContext(false);
  };

  const sendTurn = async (rawPrompt: string, override?: RetryInfo) => {
    if (!project || sendPendingRef.current || activeTurnRef.current || finalizingRef.current) {
      return;
    }
    const targetThreadId = override ? override.threadId : selectedThreadId;
    const unsavedFinal =
      stream.status === "done" &&
      stream.outcome !== null &&
      stream.threadId === targetThreadId &&
      !messages.some((entry) => entry.id === stream.outcome?.assistant_message_id);
    if (unsavedFinal) {
      return;
    }
    const text = rawPrompt.trim();
    const targetProvider = override
      ? enabledProviders.find((entry) => entry.id === override.providerId) ?? null
      : selectedProvider;
    const targetModel = (override ? override.model : model).trim();
    if (!targetProvider || !text || !targetModel) {
      return;
    }
    const contextPathsSent = override ? override.contextPaths : chipPaths;
    const allowSensitiveSent = override
      ? override.allowSensitiveContext
      : allowSensitiveContext;
    sendPendingRef.current = true;
    finalizingRef.current = null;
    pendingDeltasRef.current.clear();
    terminalRef.current.clear();
    setStreamError(null);
    setRetryInfo(null);
    setNotice(null);
    setStopping(false);

    const accepted = await sendAction.run(() =>
      api.chat.send(
        project.id,
        targetThreadId,
        targetProvider.id,
        targetModel,
        text,
        null,
        null,
        contextPathsSent,
        allowSensitiveSent,
      ),
    );
    sendPendingRef.current = false;
    if (!accepted) {
      pendingDeltasRef.current.clear();
      terminalRef.current.clear();
      return;
    }
    if (projectIdRef.current !== project.id) {
      // Le projet a changé pendant l'appel : ne pas ressusciter l'ancien tour.
      pendingDeltasRef.current.clear();
      terminalRef.current.clear();
      void api.chat.cancel(accepted.request_id).catch(() => false);
      return;
    }

    if (!override) {
      // Le contexte a été joint : on vide les chips sans toucher au message.
      setChips(clearChips());
      setAllowSensitiveContext(false);
      setSensitiveConfirm(false);
      setPrepared(null);
    }
    if (selectedThreadRef.current !== accepted.thread_id) {
      const now = new Date().toISOString();
      const stub: Thread = {
        id: accepted.thread_id,
        project_id: accepted.project_id,
        title: DEFAULT_THREAD_TITLE,
        agent_id: null,
        created_at: now,
        updated_at: now,
      };
      threads.setData((current) =>
        sortThreadsByUpdated([
          ...(current ?? []).filter((entry) => entry.id !== accepted.thread_id),
          stub,
        ]),
      );
      selectThread(accepted.thread_id, stub);
      setSearch("");
    }
    const turn: ActiveTurn = {
      requestId: accepted.request_id,
      threadId: accepted.thread_id,
      providerId: targetProvider.id,
      model: targetModel,
      prompt: text,
      localUserId: `local-${accepted.request_id}`,
      startedAt: Date.now(),
      finished: false,
      contextPaths: contextPathsSent,
      allowSensitiveContext: allowSensitiveSent,
    };
    setActiveTurn(turn);
    dispatchStream({
      type: "begin",
      requestId: accepted.request_id,
      threadId: accepted.thread_id,
    });
    const optimistic: Message = {
      id: turn.localUserId,
      thread_id: accepted.thread_id,
      role: "user",
      content: text,
      metadata: { provider: targetProvider.id, model: targetModel },
      created_at: new Date(turn.startedAt).toISOString(),
    };
    setMessages((current) => [...current, optimistic]);
    setPrompt("");
    composerRef.current?.focus();
    threads.reload();
    void refreshActive();

    const terminal = terminalRef.current.get(accepted.request_id);
    if (terminal) {
      terminalRef.current.clear();
      finalizingRef.current = accepted.request_id;
      if (terminal.type === "done") {
        await finishDone(
          {
            request_id: terminal.requestId,
            thread_id: terminal.threadId,
            outcome: terminal.outcome,
          },
          turn,
        );
      } else {
        await finishError(
          {
            request_id: terminal.requestId,
            thread_id: terminal.threadId,
            code: terminal.code,
            message: terminal.message,
          },
          turn,
        );
      }
      return;
    }
    const buffered = pendingDeltasRef.current.get(accepted.request_id) ?? "";
    pendingDeltasRef.current.clear();
    if (buffered.length > 0) {
      dispatchStream({
        type: "delta",
        requestId: accepted.request_id,
        threadId: accepted.thread_id,
        delta: buffered,
      });
    }
  };

  const handleSend = async () => {
    if (prompt.trim().length === 0) {
      return;
    }
    await sendTurn(prompt);
  };

  const handleStop = async () => {
    const turn = activeTurnRef.current;
    if (!turn || turn.finished) {
      return;
    }
    const accepted = await cancelAction.run(() => api.chat.cancel(turn.requestId));
    if (accepted === true) {
      setStopping(true);
    } else if (accepted === false) {
      setNotice({ text: t("chat.already_finished"), tone: "info" });
      void refreshActive();
    }
  };

  const handleRetry = async () => {
    if (
      !retryInfo ||
      sendPendingRef.current ||
      activeTurnRef.current ||
      finalizingRef.current
    ) {
      return;
    }
    const info = retryInfo;
    setStreamError(null);
    setRetryInfo(null);
    await sendTurn(info.prompt, info);
  };

  // ------------------------------------------------------ actions des fils
  const handleNewThread = async () => {
    if (!project) {
      return;
    }
    setNotice(null);
    const created = await createThreadAction.run(() =>
      api.history.createThread(project.id, DEFAULT_THREAD_TITLE),
    );
    if (!created || projectIdRef.current !== project.id) {
      return;
    }
    setSearch("");
    setChatMode("chat");
    threads.setData((current) =>
      sortThreadsByUpdated([
        ...(current ?? []).filter((entry) => entry.id !== created.id),
        created,
      ]),
    );
    selectThread(created.id, created);
    threads.reload();
    setPrompt("");
    composerRef.current?.focus();
  };

  const startRename = (thread: Thread) => {
    renameAction.reset();
    setRenameTarget(thread);
    setRenameTitle(displayThreadTitle(thread.title, t("chat.default_title")));
  };

  const submitRename = async () => {
    if (!renameTarget || renameAction.busy) {
      return;
    }
    const target = renameTarget;
    const trimmed = renameTitle.trim();
    if (!trimmed) {
      return;
    }
    // Conserve le sentinelle du noyau quand l'utilisateur valide le titre par défaut.
    const title =
      target.title === DEFAULT_THREAD_TITLE && trimmed === t("chat.default_title")
        ? DEFAULT_THREAD_TITLE
        : trimmed;
    const done = await renameAction.run(async () => {
      await api.history.renameThread(target.id, title);
      return true;
    });
    if (!done) {
      return;
    }
    const now = new Date().toISOString();
    setRenameTarget(null);
    setNotice({ text: t("chat.renamed"), tone: "success" });
    threads.setData((current) =>
      (current ?? []).map((entry) =>
        entry.id === target.id ? { ...entry, title, updated_at: now } : entry,
      ),
    );
    setSelectedThreadSnapshot((current) =>
      current?.id === target.id ? { ...current, title, updated_at: now } : current,
    );
    threads.reload();
  };

  const handleDelete = async () => {
    if (!toDelete || !project) {
      return;
    }
    const target = toDelete;
    const list = sortThreadsByUpdated(filterThreads(threads.data ?? [], project.id));
    const index = list.findIndex((entry) => entry.id === target.id);
    const done = await deleteAction.run(async () => {
      await api.history.deleteThread(target.id);
      return true;
    });
    if (!done) {
      return;
    }
    setToDelete(null);
    setNotice({ text: t("chat.deleted"), tone: "success" });
    const remaining = list.filter((entry) => entry.id !== target.id);
    if (selectedThreadRef.current === target.id) {
      const next =
        remaining[Math.min(Math.max(index, 0), remaining.length - 1)] ?? null;
      selectThread(next?.id ?? null, next);
    }
    threads.setData((current) => (current ?? []).filter((entry) => entry.id !== target.id));
    threads.reload();
  };

  // ------------------------------------------------------------- project
  useEffect(() => {
    const running = activeTurnRef.current;
    if (running && !running.finished) {
      // Change de projet : on interrompt réellement la génération orpheline.
      void api.chat.cancel(running.requestId).catch(() => false);
    }
    threads.setData([]);
    selectedThreadRef.current = null;
    setSelectedThreadIdState(null);
    setSelectedThreadSnapshot(null);
    setMessages([]);
    setMessagesError(null);
    setMessagesLoading(false);
    setSearch("");
    setPrompt("");
    setNotice(null);
    setStreamError(null);
    setRetryInfo(null);
    setLastTurn(null);
    setChips(clearChips());
    setAllowSensitiveContext(false);
    setSensitiveConfirm(false);
    setMentionResults([]);
    setMentionError(null);
    setMentionDismissed(false);
    setPrepared(null);
    setPrepareError(null);
    setPrepareBusy(false);
    setStopping(false);
    setActiveTurn(null);
    setChatMode("chat");
    finalizingRef.current = null;
    pendingDeltasRef.current.clear();
    terminalRef.current.clear();
    dispatchStream({ type: "reset" });
    // Réinitialisation réelle au changement de projet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project?.id]);

  const persistedFinal =
    stream.outcome !== null &&
    messages.some((entry) => entry.id === stream.outcome?.assistant_message_id);
  const generating =
    stream.status === "streaming" ||
    (activeTurn !== null && !activeTurn.finished) ||
    sendAction.busy;
  const modelReady = model.trim().length > 0;
  const finalPending =
    stream.status === "done" &&
    stream.threadId === selectedThreadId &&
    stream.outcome !== null &&
    !persistedFinal;
  const canSend =
    project !== null &&
    selectedProvider !== null &&
    modelReady &&
    prompt.trim().length > 0 &&
    !generating &&
    !finalPending;

  useEffect(() => {
    if (persistedFinal && stream.status === "done") {
      dispatchStream({ type: "reset" });
    }
  }, [persistedFinal, stream.status]);

  useEffect(() => {
    const element = messageListRef.current;
    if (element) {
      element.scrollTop = element.scrollHeight;
    }
  }, [messages.length, stream.text, stream.status, selectedThreadId, activeTurn?.requestId]);

  if (!project) {
    return (
      <>
        <PageHeader title={t("chat.title")} subtitle={t("chat.subtitle")} />
        <ProjectGate title={t("chat.title")} />
      </>
    );
  }

  const list = threads.data ?? [];
  const discoveredForProvider = selectedProvider
    ? discovered[selectedProvider.id] ?? []
    : [];
  const needsDiscovery =
    selectedProvider !== null &&
    selectedProvider.models.length === 0 &&
    discoveredForProvider.length === 0;
  const freeModelInput = selectedProvider?.kind === "ollama" && modelOptions.length === 0;
  const modelSelectOptions =
    model.trim().length > 0 && !modelOptions.some((entry) => entry.id === model)
      ? [
          {
            id: model,
            label: model,
            context_window: null,
            supports_tools: false,
            supports_vision: false,
            local: selectedProvider?.kind === "ollama",
          },
          ...modelOptions,
        ]
      : modelOptions;
  const activeThread = activeTurn?.threadId === selectedThreadId ? activeTurn : null;
  const headerProvider = activeThread
    ? enabledProviders.find((entry) => entry.id === activeThread.providerId) ?? selectedProvider
    : selectedProvider;
  const headerModel = activeThread?.model ?? model;
  const missingSecret =
    selectedProvider !== null &&
    selectedProvider.kind !== "ollama" &&
    !selectedProvider.has_secret;
  const integratedEnabled = enabledProviders.filter((entry) => !entry.custom);
  const customEnabled = enabledProviders.filter((entry) => entry.custom);

  const optimisticTurn = activeTurn?.threadId === selectedThreadId ? activeTurn : null;
  const showOptimisticUser =
    optimisticTurn !== null &&
    !messages.some(
      (entry) =>
        entry.id === optimisticTurn.localUserId ||
        (entry.role === "user" && entry.content === optimisticTurn.prompt),
    );
  const showStreaming =
    stream.status === "streaming" && stream.threadId === selectedThreadId;
  const showFinal =
    stream.status === "done" &&
    stream.threadId === selectedThreadId &&
    stream.outcome !== null &&
    !persistedFinal;
  const finalMessage: Message | null =
    showFinal && stream.outcome && stream.threadId
      ? messageFromOutcome(
          stream.outcome,
          stream.threadId,
          headerProvider?.id ?? "",
          headerModel,
        )
      : null;
  const hasVisibleMessages =
    messages.length > 0 || showOptimisticUser || showStreaming || finalMessage !== null;

  const renderMessage = (message: Message, streaming = false) => {
    const metadata = readMessageMetadata(message.metadata);
    const usage = metadata.usage;
    const hasUsage =
      usage !== null && (usage.input_tokens !== null || usage.output_tokens !== null);
    return (
      <li
        key={message.id}
        className={`chat-message chat-message-${message.role}${
          streaming ? " chat-message-streaming" : ""
        }`}
      >
        <div className="chat-message-head">
          <Badge tone={roleTone(message.role)}>{roleLabel(t, message.role)}</Badge>
          <span className="small muted">{formatDate(message.created_at, locale)}</span>
        </div>
        <div className="chat-message-content prewrap">
          {message.content}
          {streaming ? <span className="chat-cursor" aria-hidden="true" /> : null}
        </div>
        {metadata.cancelled ? (
          <div className="chat-message-tags">
            <Badge tone="warning">{t("chat.cancelled")}</Badge>
          </div>
        ) : null}
        <div className="chat-message-meta">
          {hasUsage && usage ? (
            <>
              {usage.input_tokens !== null ? (
                <span>{t("chat.token_input_short", { count: usage.input_tokens })}</span>
              ) : null}
              {usage.output_tokens !== null ? (
                <span>{t("chat.token_output_short", { count: usage.output_tokens })}</span>
              ) : null}
            </>
          ) : null}
          {metadata.elapsedMs !== null ? (
            <span>{t("chat.elapsed", { ms: metadata.elapsedMs })}</span>
          ) : null}
          {metadata.finishReason ? (
            <span>{t("chat.finish_reason", { reason: metadata.finishReason })}</span>
          ) : null}
        </div>
      </li>
    );
  };

  const lastTurnLabel = lastTurn
    ? t(
        lastTurn.cancelled
          ? "chat.last_turn_cancelled"
          : totalTokens(lastTurn.usage) === null
            ? "chat.last_turn_no_usage"
            : "chat.last_turn_stats",
        totalTokens(lastTurn.usage) === null
          ? { ms: lastTurn.elapsedMs }
          : { ms: lastTurn.elapsedMs, tokens: totalTokens(lastTurn.usage) ?? 0 },
      )
    : t("chat.last_turn_none");

  const maxChatWidth = Math.max(
    MIN_CONVERSATION_WIDTH,
    Math.min(
      maxConversationWidth(ws.bounds),
      ws.bounds.width -
        (ws.layout.explorer_visible ? MIN_EXPLORER_WIDTH : 0) -
        MIN_EDITOR_WIDTH -
        14,
    ),
  );
  const maxExplorer = Math.max(
    MIN_EXPLORER_WIDTH,
    Math.min(
      maxExplorerWidth(ws.bounds),
      ws.bounds.width - ws.layout.conversation_width - MIN_EDITOR_WIDTH - 14,
    ),
  );
  const maxBottom = maxBottomHeight(ws.bounds);

  return (
    <>
      <PageHeader
        title={t("workspace.title")}
        subtitle={project.path}
        actions={
          <>
            <button
              type="button"
              className="ghost"
              onClick={() => {
                threads.reload();
                providers.reload();
                void refreshActive();
              }}
            >
              <RefreshCw size={15} aria-hidden="true" />
              {t("common.refresh")}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={ws.toggleExplorer}
              aria-pressed={ws.layout.explorer_visible}
            >
              <PanelLeft size={15} aria-hidden="true" />
              {ws.layout.explorer_visible
                ? t("workspace.hide_explorer")
                : t("workspace.show_explorer")}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={ws.toggleBottom}
              aria-expanded={ws.layout.bottom_open}
              aria-controls="workspace-bottom-panel"
            >
              <PanelBottom size={15} aria-hidden="true" />
              {ws.layout.bottom_open
                ? t("workspace.hide_bottom")
                : t("workspace.show_bottom")}
            </button>
            <button type="button" className="ghost" onClick={ws.resetLayout}>
              <RotateCcw size={15} aria-hidden="true" />
              {t("workspace.reset_layout")}
            </button>
          </>
        }
      />

      <div className="chat-resource-bar" role="status">
        <span className="chat-resource-item">
          <Activity size={13} aria-hidden="true" />
          {t("chat.active_generations", { count: activeCount ?? "—" })}
        </span>
        <span className="chat-resource-item" title={t("chat.last_turn")}>
          {lastTurnLabel}
        </span>
        <span className="chat-resource-item" title={project.path}>
          {t("workspace.project_label", { name: project.name })}
        </span>
        {activeTurn && !activeTurn.finished ? (
          <button
            type="button"
            className="ghost small danger-text"
            onClick={() => void handleStop()}
            disabled={stopping || cancelAction.busy}
            aria-busy={cancelAction.busy}
          >
            {cancelAction.busy || stopping ? <Spinner /> : <Ban size={13} aria-hidden="true" />}
            {stopping ? t("chat.stopping") : t("chat.stop")}
          </button>
        ) : null}
      </div>

      {providers.error ? (
        <ErrorBanner error={providers.error} onRetry={providers.reload} />
      ) : null}
      {threads.error ? <ErrorBanner error={threads.error} onRetry={threads.reload} /> : null}
      {deltaListenerError ? <ErrorBanner error={deltaListenerError} /> : null}
      {doneListenerError ? <ErrorBanner error={doneListenerError} /> : null}
      {errorListenerError ? <ErrorBanner error={errorListenerError} /> : null}
      {createThreadAction.error ? <ErrorBanner error={createThreadAction.error} /> : null}
      {renameAction.error ? <ErrorBanner error={renameAction.error} /> : null}
      {deleteAction.error ? <ErrorBanner error={deleteAction.error} /> : null}
      {sendAction.error ? <ErrorBanner error={sendAction.error} /> : null}
      {cancelAction.error ? <ErrorBanner error={cancelAction.error} /> : null}
      {saveDefault.error ? (
        <ErrorBanner error={saveDefault.error} onDismiss={() => saveDefault.setError(null)} />
      ) : null}
      {discover.error ? (
        <ErrorBanner error={discover.error} onDismiss={() => discover.setError(null)} />
      ) : null}
      {streamError ? (
        <ErrorBanner
          error={streamError}
          onRetry={retryInfo ? () => void handleRetry() : undefined}
          onDismiss={() => {
            setStreamError(null);
            setRetryInfo(null);
          }}
        />
      ) : null}
      {notice ? <Notice tone={notice.tone}>{notice.text}</Notice> : null}
      {ws.layoutError ? (
        <ErrorBanner error={ws.layoutError} onDismiss={ws.dismissLayoutError} />
      ) : null}

      {!ws.hydrated ? (
        <LoadingRow label={t("workspace.loading")} />
      ) : (
      <div className="workspace-shell" ref={ws.containerRef}>
        <div
          className="workspace-main"
          style={{
            height: ws.layout.bottom_open
              ? `calc(100% - ${ws.layout.bottom_height}px - 10px)`
              : "100%",
          }}
        >
          <div
            className={
              ws.layout.conversation_width < 560 ? "workspace-chat compact" : "workspace-chat"
            }
            style={{ width: ws.layout.conversation_width }}
          >
            <div className="chat-layout">
        <section className="card chat-threads" aria-labelledby="chat-threads-title">
          <div className="section-heading">
            <h2 id="chat-threads-title">{t("chat.threads")}</h2>
            <span className="small muted">
              {t("chat.thread_count", { count: list.length })}
            </span>
          </div>

          <div className="chat-search">
            <Search size={14} aria-hidden="true" className="chat-search-icon" />
            <input
              className="input"
              type="search"
              value={search}
              placeholder={t("chat.search_placeholder")}
              aria-label={t("chat.search_placeholder")}
              onChange={(event) => setSearch(event.target.value)}
            />
            {search.length > 0 ? (
              <button
                type="button"
                className="icon-button chat-search-clear"
                aria-label={t("chat.search_clear")}
                onClick={() => setSearch("")}
              >
                <X size={14} />
              </button>
            ) : null}
            {threads.loading ? <Spinner /> : null}
          </div>

          <button
            type="button"
            className="primary chat-new-thread"
            onClick={() => void handleNewThread()}
            disabled={createThreadAction.busy}
            aria-busy={createThreadAction.busy}
          >
            {createThreadAction.busy ? <Spinner /> : <Plus size={14} aria-hidden="true" />}
            {t("chat.new_thread")}
          </button>

          {threads.loading && threads.data === null ? (
            <LoadingRow label={t("chat.threads_loading")} />
          ) : list.length === 0 ? (
            <EmptyState
              icon={<MessageSquare size={20} aria-hidden="true" />}
              title={
                debouncedSearch.trim().length > 0 && !threads.loading
                  ? t("chat.no_results")
                  : t("chat.no_threads")
              }
              description={<p>{t("chat.no_threads_hint")}</p>}
            />
          ) : (
            <ul className="chat-thread-list">
              {list.map((thread) => {
                const selected = thread.id === selectedThreadId;
                const live = activeTurn?.threadId === thread.id && !activeTurn.finished;
                return (
                  <li
                    key={thread.id}
                    className={selected ? "chat-thread-row selected" : "chat-thread-row"}
                  >
                    <button
                      type="button"
                      className="chat-thread-button"
                      onClick={() => {
                        setChatMode("chat");
                        selectThread(thread.id, thread);
                      }}
                      aria-current={selected ? "true" : undefined}
                    >
                      <span className="chat-thread-title">
                        {displayThreadTitle(thread.title, t("chat.default_title"))}
                      </span>
                      <span className="chat-thread-meta">
                        <span
                          title={formatDate(thread.updated_at, locale)}
                          className="small muted"
                        >
                          {formatRelativeTime(thread.updated_at, locale)}
                        </span>
                        {live ? (
                          <span className="inline-status">
                            <Spinner />
                            <span className="small">{t("chat.active_thread_badge")}</span>
                          </span>
                        ) : null}
                      </span>
                    </button>
                    <details className="chat-thread-menu">
                      <summary
                        className="icon-button"
                        aria-label={t("chat.thread_actions")}
                        title={t("chat.thread_actions")}
                      >
                        <MoreHorizontal size={15} />
                      </summary>
                      <div className="chat-thread-menu-pop">
                        <button
                          type="button"
                          onClick={(event) => {
                            event.currentTarget.closest("details")?.removeAttribute("open");
                            startRename(thread);
                          }}
                        >
                          <Pencil size={13} aria-hidden="true" />
                          {t("chat.rename")}
                        </button>
                        <button
                          type="button"
                          className="danger-text"
                          onClick={(event) => {
                            event.currentTarget.closest("details")?.removeAttribute("open");
                            deleteAction.reset();
                            setToDelete(thread);
                          }}
                        >
                          <Trash2 size={13} aria-hidden="true" />
                          {t("chat.delete")}
                        </button>
                      </div>
                    </details>
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        <div className="chat-conversation-column">
          <div className="chat-mode-bar">
            <ChatModeSwitch mode={chatMode} onChange={setChatMode} />
            {chatMode === "chat" && agentSession.active ? (
              <button
                type="button"
                className="ghost small agent-active-indicator"
                onClick={() => setChatMode("agent")}
                title={t("agent_mode.back_to_agent")}
              >
                <Bot size={13} aria-hidden="true" />
                {t("agent_mode.active_indicator", {
                  phase: t(agentPhaseLabelKey(agentSession.phase)),
                })}
              </button>
            ) : null}
            {chatMode === "agent" ? (
              <span className="small muted chat-mode-hint">{t("agent_mode.mode_hint")}</span>
            ) : null}
          </div>

          {chatMode === "agent" ? (
            <AgentPanel projectId={project.id} controller={agentSession} workspace={ws} />
          ) : providers.loading ? (
          <section className="card chat-conversation">
            <LoadingRow label={t("chat.providers_loading")} />
          </section>
        ) : enabledProviders.length === 0 ? (
          <section className="card chat-conversation">
            <EmptyState
              icon={<MessagesSquare size={22} aria-hidden="true" />}
              title={t("chat.no_providers")}
              description={
                <>
                  <p>{t("chat.no_providers_hint")}</p>
                  {providers.error ? <p>{t("chat.no_providers_error")}</p> : null}
                </>
              }
              action={
                <button type="button" className="primary" onClick={() => navigate("models")}>
                  {t("chat.open_models")}
                </button>
              }
            />
          </section>
        ) : (
          <section className="card chat-conversation" aria-labelledby="chat-conversation-title">
            <header className="chat-conversation-head">
              <div className="chat-conversation-heading">
                <h2 id="chat-conversation-title">
                  {selectedThread
                    ? displayThreadTitle(selectedThread.title, t("chat.default_title"))
                    : t("chat.default_title")}
                </h2>
                <div className="chat-badges">
                  <Badge tone="accent">
                    {headerProvider?.label ?? t("chat.no_provider")}
                  </Badge>
                  {headerModel ? <Badge tone="info">{headerModel}</Badge> : null}
                  {selectedThread ? (
                    <span className="small muted">
                      {formatRelativeTime(selectedThread.updated_at, locale)}
                    </span>
                  ) : null}
                  {generating ? (
                    <Badge tone="warning">
                      <Spinner />
                      {stopping ? t("chat.stopping") : t("chat.generating")}
                    </Badge>
                  ) : null}
                </div>
              </div>
              <div className="button-row compact">
                {activeThread && !activeThread.finished ? (
                  <button
                    type="button"
                    className="danger small"
                    onClick={() => void handleStop()}
                    disabled={stopping || cancelAction.busy}
                    aria-busy={cancelAction.busy}
                  >
                    {cancelAction.busy || stopping ? <Spinner /> : <Ban size={13} aria-hidden="true" />}
                    {stopping ? t("chat.stopping") : t("chat.stop")}
                  </button>
                ) : null}
                <button
                  type="button"
                  className="ghost small"
                  onClick={() => {
                    if (selectedThreadId) {
                      void loadMessages(selectedThreadId);
                    }
                  }}
                  disabled={!selectedThreadId || messagesLoading}
                  aria-busy={messagesLoading}
                >
                  {messagesLoading ? <Spinner /> : <RefreshCw size={13} aria-hidden="true" />}
                  {t("chat.refresh_messages")}
                </button>
              </div>
            </header>

            <div className="chat-controls">
              <Field label={t("chat.provider")} htmlFor="chat-provider">
                <select
                  id="chat-provider"
                  className="input"
                  value={providerId}
                  onChange={(event) => setProviderId(event.target.value)}
                >
                  {providerId ? null : <option value="">{t("chat.choose_provider")}</option>}
                  {integratedEnabled.length > 0 ? (
                    <optgroup label={t("providers.group_integrated")}>
                      {integratedEnabled.map((entry) => (
                        <option key={entry.id} value={entry.id}>
                          {entry.label}
                        </option>
                      ))}
                    </optgroup>
                  ) : null}
                  {customEnabled.length > 0 ? (
                    <optgroup label={t("providers.group_custom")}>
                      {customEnabled.map((entry) => (
                        <option key={entry.id} value={entry.id}>
                          {entry.label}
                        </option>
                      ))}
                    </optgroup>
                  ) : null}
                  {disabledProviders.length > 0 ? (
                    <optgroup label={t("providers.group_disabled")}>
                      {disabledProviders.map((entry) => (
                        <option key={entry.id} value={entry.id} disabled>
                          {entry.label}
                        </option>
                      ))}
                    </optgroup>
                  ) : null}
                </select>
                {selectedProvider || disabledProviders.length > 0 ? (
                  <div className="inline-status chat-provider-badges">
                    {selectedProvider ? (
                      <Badge tone={selectedProvider.custom ? "info" : "accent"}>
                        {selectedProvider.custom
                          ? t("providers.badge_custom")
                          : t("providers.badge_integrated")}
                      </Badge>
                    ) : null}
                    {disabledProviders.map((entry) => (
                      <Badge
                        key={entry.id}
                        tone="warning"
                        title={t("providers.disabled_select_hint")}
                      >
                        {entry.label} · {t("providers.badge_inactive")}
                      </Badge>
                    ))}
                  </div>
                ) : null}
              </Field>
              <Field
                label={t("chat.model")}
                htmlFor={freeModelInput ? "chat-model-free" : "chat-model"}
                hint={
                  freeModelInput
                    ? t("chat.model_free_hint")
                    : modelOptions.length === 0
                      ? t("chat.no_model_hint")
                      : t("chat.models_hint")
                }
              >
                <div className="input-row chat-model-row">
                  {freeModelInput ? (
                    <input
                      id="chat-model-free"
                      className="input"
                      value={model}
                      placeholder={t("chat.model_free_placeholder")}
                      onChange={(event) => setModel(event.target.value)}
                      onBlur={() => {
                        if (selectedProvider) {
                          void persistDefaultModel(selectedProvider.id, model);
                        }
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          if (selectedProvider) {
                            void persistDefaultModel(selectedProvider.id, model);
                          }
                          event.currentTarget.blur();
                        }
                      }}
                    />
                  ) : (
                    <select
                      id="chat-model"
                      className="input"
                      value={model}
                      disabled={modelSelectOptions.length === 0}
                      onChange={(event) => handleModelChange(event.target.value)}
                    >
                      {model ? null : <option value="">{t("chat.choose_model")}</option>}
                      {modelSelectOptions.map((entry) => (
                        <option key={entry.id} value={entry.id}>
                          {entry.label || entry.id}
                        </option>
                      ))}
                    </select>
                  )}
                  {needsDiscovery ? (
                    <button
                      type="button"
                      className="ghost small"
                      onClick={() => void discoverModels()}
                      disabled={discover.busy}
                      aria-busy={discover.busy}
                    >
                      {discover.busy ? <Spinner /> : <RefreshCw size={13} aria-hidden="true" />}
                      {discover.busy ? t("chat.discovering") : t("chat.discover")}
                    </button>
                  ) : null}
                  {selectedProvider?.custom ? (
                    <Badge tone="info">{t("providers.badge_custom")}</Badge>
                  ) : null}
                </div>
              </Field>
            </div>

            {missingSecret ? (
              <Notice tone="warning">{t("chat.provider_requires_key")}</Notice>
            ) : null}
            {messagesError ? (
              <ErrorBanner
                error={messagesError}
                onRetry={() => {
                  if (selectedThreadId) {
                    void loadMessages(selectedThreadId);
                  }
                }}
              />
            ) : null}

            <div className="chat-message-area">
              {messagesLoading && !hasVisibleMessages ? (
                <LoadingRow label={t("chat.loading_messages")} />
              ) : (
                <ul
                  className="chat-message-list"
                  ref={messageListRef}
                  aria-label={t("chat.messages")}
                >
                  {messages.map((entry) => renderMessage(entry))}
                  {showOptimisticUser && optimisticTurn
                    ? renderMessage({
                        id: optimisticTurn.localUserId,
                        thread_id: optimisticTurn.threadId,
                        role: "user",
                        content: optimisticTurn.prompt,
                        metadata: {
                          provider: optimisticTurn.providerId,
                          model: optimisticTurn.model,
                        },
                        created_at: new Date(optimisticTurn.startedAt).toISOString(),
                      })
                    : null}
                  {showStreaming ? (
                    <li
                      className="chat-message chat-message-assistant chat-message-streaming"
                      aria-live="polite"
                    >
                      <div className="chat-message-head">
                        <Badge tone="info">{t("chat.role_assistant")}</Badge>
                        <span className="small muted">{t("chat.stream_cursor")}</span>
                      </div>
                      <div className="chat-message-content prewrap">
                        {stream.text}
                        <span className="chat-cursor" aria-hidden="true" />
                      </div>
                    </li>
                  ) : null}
                  {finalMessage ? renderMessage(finalMessage) : null}
                </ul>
              )}
              {!messagesLoading && !hasVisibleMessages ? (
                <EmptyState
                  icon={<MessagesSquare size={22} aria-hidden="true" />}
                  title={t("chat.no_messages")}
                  description={<p>{t("chat.no_messages_hint")}</p>}
                />
              ) : null}
            </div>

            <div className="context-bar" aria-labelledby="context-bar-title">
              <div className="context-bar-head">
                <span className="context-bar-title" id="context-bar-title">
                  <AtSign size={13} aria-hidden="true" />
                  {t("workspace.project_label", { name: project.name })}
                </span>
                {chipPaths.length > 0 ? (
                  <button type="button" className="ghost small" onClick={clearContext}>
                    <Trash2 size={12} aria-hidden="true" />
                    {t("workspace.clear_context")}
                  </button>
                ) : null}
              </div>
              <p className="small muted">{t("workspace.context_hint")}</p>
              {chips.length > 0 ? (
                <ul className="context-chips" aria-label={t("workspace.context_files")}>
                  {chips.map((chip) => (
                    <li key={chip.path} className="context-chip">
                      <FileCode2 size={12} aria-hidden="true" />
                      <span className="mono small" title={chip.path}>
                        {chip.path}
                      </span>
                      <span className="small muted">
                        {chip.size !== null
                          ? formatBytes(chip.size)
                          : t("workspace.size_unknown")}
                      </span>
                      <button
                        type="button"
                        className="icon-button"
                        aria-label={t("workspace.remove_context_file", { path: chip.path })}
                        title={t("workspace.remove_context_file", { path: chip.path })}
                        onClick={() => removeContextChip(chip.path)}
                      >
                        <X size={12} />
                      </button>
                    </li>
                  ))}
                </ul>
              ) : null}
              {allowSensitiveContext ? (
                <div className="context-sensitive-flag">
                  <Badge tone="warning">
                    <ShieldAlert size={12} aria-hidden="true" />
                    {t("workspace.sensitive_allowed")}
                  </Badge>
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => setAllowSensitiveContext(false)}
                  >
                    {t("workspace.sensitive_revoke")}
                  </button>
                </div>
              ) : null}
              {ws.settingsError ? (
                <p className="small danger-text">
                  {t("workspace.settings_error", { message: ws.settingsError.message })}
                </p>
              ) : null}
              {prepareBusy ? (
                <p className="small muted" role="status">
                  <Spinner /> {t("workspace.context_preparing")}
                </p>
              ) : null}
              {prepareError ? (
                <ErrorBanner
                  error={prepareError}
                  onRetry={() => setPrepareNonce((value) => value + 1)}
                />
              ) : null}
              {prepared ? (
                <div className="context-preview" aria-live="polite">
                  <p className="small">
                    {t("workspace.context_included", { count: prepared.files.length })} ·{" "}
                    {t("workspace.context_bytes", {
                      bytes: formatBytes(prepared.total_bytes),
                    })}
                    {ws.settings
                      ? ` · ${t("workspace.context_budget", {
                          max: formatBytes(ws.settings.max_total_bytes),
                          files: ws.settings.max_files,
                        })}`
                      : ""}
                  </p>
                  {prepared.files.length === 0 ? (
                    <p className="small muted">{t("workspace.context_none")}</p>
                  ) : (
                    <ul className="context-preview-list">
                      {prepared.files.map((file) => (
                        <li key={file.path}>
                          <span className="mono small" title={file.path}>
                            {file.path}
                          </span>
                          <span className="small muted">{formatBytes(file.bytes)}</span>
                          {file.truncated ? (
                            <Badge tone="warning">{t("workspace.context_truncated")}</Badge>
                          ) : null}
                          {file.sensitive ? (
                            <Badge tone="warning">{t("workspace.context_sensitive")}</Badge>
                          ) : null}
                        </li>
                      ))}
                    </ul>
                  )}
                  {prepared.skipped.length > 0 ? (
                    <div className="context-skipped">
                      <p className="small">
                        <strong>
                          {t("workspace.context_skipped_title", {
                            count: prepared.skipped.length,
                          })}
                        </strong>
                      </p>
                      <ul className="context-preview-list">
                        {prepared.skipped.map((entry) => (
                          <li key={entry.path}>
                            <span className="mono small" title={entry.path}>
                              {entry.path}
                            </span>
                            <span className="small muted">{entry.reason}</span>
                          </li>
                        ))}
                      </ul>
                      {sensitiveSkipped.length > 0 && !allowSensitiveContext ? (
                        <button
                          type="button"
                          className="ghost small"
                          onClick={() => setSensitiveConfirm(true)}
                        >
                          <ShieldAlert size={12} aria-hidden="true" />
                          {t("workspace.include_sensitive")}
                        </button>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>

            <div className="chat-composer">
              {mentionOpen ? (
                <div
                  className="mention-pop"
                  role="listbox"
                  id="chat-mention-list"
                  aria-label={t("workspace.mention_label")}
                >
                  <div className="mention-pop-head">
                    <span className="small muted">{t("workspace.mention_hint")}</span>
                    {mentionLoading ? <Spinner /> : null}
                  </div>
                  {mentionError ? (
                    <p className="small danger-text">{mentionError.message}</p>
                  ) : null}
                  {!mentionLoading && mentionResults.length === 0 && !mentionError ? (
                    <p className="small muted">
                      {t("workspace.mention_empty", { query: mentionToken?.query ?? "" })}
                    </p>
                  ) : null}
                  {mentionResults.map((match, index) => (
                    <button
                      key={match.path}
                      type="button"
                      role="option"
                      aria-selected={index === mentionIndex}
                      className={
                        index === mentionIndex
                          ? "mention-option active"
                          : "mention-option"
                      }
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={() => chooseMention(match)}
                    >
                      <FileCode2 size={13} aria-hidden="true" />
                      <span className="mono small" title={match.path}>
                        {match.path}
                      </span>
                      {match.excluded ? (
                        <Badge tone="warning">{t("workspace.excluded")}</Badge>
                      ) : null}
                    </button>
                  ))}
                </div>
              ) : null}
              <Field label={t("chat.prompt_label")} htmlFor="chat-prompt" hint={t("chat.prompt_hint")}>
                <textarea
                  id="chat-prompt"
                  ref={composerRef}
                  className="input textarea"
                  rows={4}
                  value={prompt}
                  placeholder={t("chat.prompt_placeholder")}
                  aria-label={t("chat.prompt_label")}
                  aria-autocomplete="list"
                  aria-expanded={mentionOpen}
                  aria-controls={mentionOpen ? "chat-mention-list" : undefined}
                  disabled={generating || selectedProvider === null}
                  onChange={(event) => {
                    setPrompt(event.target.value);
                    setMentionDismissed(false);
                  }}
                  onKeyDown={(event) => {
                    if (handleMentionKeyDown(event)) {
                      return;
                    }
                    if (
                      event.key === "Enter" &&
                      !event.shiftKey &&
                      !event.nativeEvent.isComposing
                    ) {
                      event.preventDefault();
                      void handleSend();
                    }
                  }}
                />
              </Field>
              <div className="chat-composer-footer">
                <span className="small muted">{t("chat.char_count", { count: prompt.length })}</span>
                <div className="button-row compact">
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void handleSend()}
                    disabled={!canSend}
                    aria-busy={sendAction.busy}
                  >
                    {sendAction.busy ? <Spinner /> : <Send size={14} aria-hidden="true" />}
                    {t("chat.send")}
                  </button>
                  {activeTurn && !activeTurn.finished ? (
                    <button
                      type="button"
                      className="danger"
                      onClick={() => void handleStop()}
                      disabled={stopping || cancelAction.busy}
                      aria-busy={cancelAction.busy}
                    >
                      {cancelAction.busy || stopping ? <Spinner /> : <Ban size={14} aria-hidden="true" />}
                      {stopping ? t("chat.stopping") : t("chat.stop")}
                    </button>
                  ) : null}
                </div>
              </div>
            </div>
          </section>
        )}
        </div>
            </div>
          </div>

          <Resizer
            orientation="vertical"
            value={ws.layout.conversation_width}
            min={MIN_CONVERSATION_WIDTH}
            max={Math.max(MIN_CONVERSATION_WIDTH, maxChatWidth)}
            label={t("workspace.resize_conversation")}
            onChange={(value) => ws.updateLayout({ conversation_width: value })}
          />

          {ws.layout.explorer_visible ? (
            <>
              <FileExplorer
                projectId={project.id}
                projectName={project.name}
                activePath={ws.activePath}
                width={ws.layout.explorer_width}
                onOpen={(path) => void ws.openFile(path)}
                onHide={ws.toggleExplorer}
              />
              <Resizer
                orientation="vertical"
                value={ws.layout.explorer_width}
                min={MIN_EXPLORER_WIDTH}
                max={Math.max(MIN_EXPLORER_WIDTH, maxExplorer)}
                label={t("workspace.resize_explorer")}
                onChange={(value) => ws.updateLayout({ explorer_width: value })}
              />
            </>
          ) : null}

          <WorkspaceEditor workspace={ws} />
        </div>

        {ws.layout.bottom_open ? (
          <Resizer
            orientation="horizontal"
            value={ws.layout.bottom_height}
            min={MIN_BOTTOM_HEIGHT}
            max={Math.max(MIN_BOTTOM_HEIGHT, maxBottom)}
            label={t("workspace.resize_bottom")}
            onChange={(value) => ws.updateLayout({ bottom_height: value })}
          />
        ) : null}
        <BottomPanel
          workspace={ws}
          projectId={project.id}
          activeTab={ws.layout.bottom_tab}
          open={ws.layout.bottom_open}
          onTabChange={ws.setBottomTab}
          onClose={ws.toggleBottom}
        />
      </div>
      )}

      <Modal
        open={renameTarget !== null}
        title={t("chat.rename_title")}
        onClose={() => {
          if (!renameAction.busy) {
            setRenameTarget(null);
          }
        }}
        footer={
          <>
            <button
              type="button"
              className="ghost"
              onClick={() => setRenameTarget(null)}
              disabled={renameAction.busy}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void submitRename()}
              disabled={renameAction.busy || renameTitle.trim().length === 0}
              aria-busy={renameAction.busy}
            >
              {renameAction.busy ? <Spinner /> : null}
              {t("common.save")}
            </button>
          </>
        }
      >
        <Field label={t("chat.rename_label")} htmlFor="chat-rename-title" required>
          <input
            id="chat-rename-title"
            className="input"
            value={renameTitle}
            placeholder={t("chat.rename_placeholder")}
            onChange={(event) => setRenameTitle(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void submitRename();
              }
            }}
          />
        </Field>
      </Modal>

      <ConfirmDialog
        open={toDelete !== null}
        title={t("chat.delete_confirm_title", {
          title: toDelete ? displayThreadTitle(toDelete.title, t("chat.default_title")) : "",
        })}
        description={<p>{t("chat.delete_confirm_text")}</p>}
        confirmLabel={t("common.delete")}
        onCancel={() => setToDelete(null)}
        onConfirm={() => void handleDelete()}
        busy={deleteAction.busy}
        danger
      />

      <ConfirmDialog
        open={sensitiveConfirm}
        title={t("workspace.sensitive_confirm_title")}
        description={
          <>
            <p>{t("workspace.sensitive_confirm_text", { count: sensitiveSkipped.length })}</p>
            <p className="small muted">{t("workspace.sensitive_confirm_hint")}</p>
          </>
        }
        confirmLabel={t("workspace.include_sensitive")}
        onCancel={() => setSensitiveConfirm(false)}
        onConfirm={() => {
          setSensitiveConfirm(false);
          setPrepared(null);
          setAllowSensitiveContext(true);
        }}
        danger
      />
    </>
  );
}
