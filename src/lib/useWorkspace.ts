import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";
import {
  api,
  toAppError,
  type AppError,
  type ContextSettings,
  type DiffPreview,
  type PendingDiff,
} from "./api";
import {
  DEFAULT_LAYOUT,
  clampLayout,
  normalizeBottomTab,
  parseLayout,
  serializeLayout,
  type BottomTab,
  type LayoutBounds,
  type WorkspaceLayout,
} from "./layout";
import {
  closeTabState,
  createTab,
  isTabDirty,
  markTabSaved,
  openTabState,
  planCloseTab,
  reloadTab,
  restoreTabs,
  updateTabContent,
  activateTabState,
  type EditorTab,
  type RestoredTabFile,
} from "./tabs";
import { useApp } from "./store";

/**
 * Borne de secours de l'éditeur quand `context_settings` n'expose pas
 * `max_editor_bytes` (2 Mio, la valeur par défaut du noyau). `file_read`
 * renvoie aussi `truncated`, qui reste la garde finale.
 */
export const FALLBACK_EDITOR_BYTES = 2 * 1024 * 1024;

/** Lignes maximales restaurées au montage pour ne pas bloquer le WebView. */
const MAX_RESTORED_TABS = 24;

export interface WorkspaceNotice {
  text: string;
  tone: "info" | "success" | "warning" | "error";
}

export interface SaveConflict {
  error: AppError;
  path: string;
}

export interface WorkspaceController {
  layout: WorkspaceLayout;
  bounds: LayoutBounds;
  hydrated: boolean;
  settings: ContextSettings | null;
  settingsError: AppError | null;

  tabs: EditorTab[];
  activePath: string | null;
  activeTab: EditorTab | null;
  dirty: boolean;

  fileError: AppError | null;
  saveError: AppError | null;
  saveConflict: SaveConflict | null;
  diff: DiffPreview | null;
  diffError: AppError | null;
  diffBusy: boolean;
  saveBusy: boolean;
  fileBusy: boolean;
  layoutError: AppError | null;
  pendingDiffs: PendingDiff[];
  pendingDiffError: AppError | null;
  pendingDiffBusy: boolean;
  closeTarget: string | null;
  notice: WorkspaceNotice | null;

  containerRef: RefObject<HTMLDivElement>;

  updateLayout: (patch: Partial<WorkspaceLayout>) => void;
  resetLayout: () => void;
  toggleExplorer: () => void;
  toggleBottom: () => void;
  setBottomTab: (tab: BottomTab) => void;

  openFile: (path: string) => Promise<void>;
  activate: (path: string) => void;
  updateContent: (path: string, content: string) => void;
  save: (force?: boolean, path?: string | null) => Promise<void>;
  reloadActive: () => Promise<void>;
  requestClose: (path: string) => void;
  confirmClose: () => void;
  cancelClose: () => void;

  showDiff: () => Promise<void>;
  closeDiff: () => void;
  acceptDiff: () => Promise<void>;
  rejectDiff: () => Promise<void>;

  refreshPendingDiffs: () => Promise<void>;
  acceptPendingDiff: (id: string) => Promise<void>;
  rejectPendingDiff: (id: string) => Promise<void>;

  dismissFileError: () => void;
  dismissSaveError: () => void;
  dismissDiffError: () => void;
  dismissNotice: () => void;
  dismissLayoutError: () => void;
}

function sameLayout(left: WorkspaceLayout, right: WorkspaceLayout): boolean {
  return (
    left.conversation_width === right.conversation_width &&
    left.explorer_width === right.explorer_width &&
    left.explorer_visible === right.explorer_visible &&
    left.bottom_height === right.bottom_height &&
    left.bottom_open === right.bottom_open &&
    left.bottom_tab === right.bottom_tab &&
    left.project_id === right.project_id &&
    left.active_tab === right.active_tab &&
    left.tabs.length === right.tabs.length
  );
}

/**
 * État complet du workspace : disposition persistée, onglets relus depuis le
 * disque, sauvegardes vérifiées par empreinte, diff et modifications en
 * attente. Toutes les commandes sont asynchrones ; aucun état n'est simulé.
 */
export function useWorkspace(projectId: string | null): WorkspaceController {
  const { t } = useApp();
  const [layout, setLayoutState] = useState<WorkspaceLayout>(DEFAULT_LAYOUT);
  const [bounds, setBounds] = useState<LayoutBounds>({ width: 1440, height: 900 });
  const [hydrated, setHydrated] = useState(false);
  const [settings, setSettings] = useState<ContextSettings | null>(null);
  const [settingsError, setSettingsError] = useState<AppError | null>(null);
  const [tabState, setTabState] = useState<{ tabs: EditorTab[]; activePath: string | null }>({
    tabs: [],
    activePath: null,
  });
  const [fileError, setFileError] = useState<AppError | null>(null);
  const [saveError, setSaveError] = useState<AppError | null>(null);
  const [saveConflict, setSaveConflict] = useState<SaveConflict | null>(null);
  const [diff, setDiff] = useState<DiffPreview | null>(null);
  const [diffError, setDiffError] = useState<AppError | null>(null);
  const [diffBusy, setDiffBusy] = useState(false);
  const [saveBusy, setSaveBusy] = useState(false);
  const [fileBusy, setFileBusy] = useState(false);
  const [layoutError, setLayoutError] = useState<AppError | null>(null);
  const [pendingDiffs, setPendingDiffs] = useState<PendingDiff[]>([]);
  const [pendingDiffError, setPendingDiffError] = useState<AppError | null>(null);
  const [pendingDiffBusy, setPendingDiffBusy] = useState(false);
  const [closeTarget, setCloseTarget] = useState<string | null>(null);
  const [notice, setNotice] = useState<WorkspaceNotice | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);
  const projectRef = useRef<string | null>(projectId);
  projectRef.current = projectId;
  const tabsRef = useRef(tabState.tabs);
  tabsRef.current = tabState.tabs;
  const activePathRef = useRef(tabState.activePath);
  activePathRef.current = tabState.activePath;
  const boundsRef = useRef(bounds);
  boundsRef.current = bounds;
  const settingsRef = useRef<ContextSettings | null>(settings);
  settingsRef.current = settings;
  const diffRef = useRef<DiffPreview | null>(diff);
  diffRef.current = diff;
  const scopeToken = useRef(0);
  const saveBusyRef = useRef(false);

  const editorLimit = settings?.max_editor_bytes ?? FALLBACK_EDITOR_BYTES;
  const editorLimitRef = useRef(editorLimit);
  editorLimitRef.current = editorLimit;

  const activeTab = useMemo(
    () => tabState.tabs.find((tab) => tab.path === tabState.activePath) ?? null,
    [tabState],
  );
  const dirty = activeTab !== null && isTabDirty(activeTab);

  // ---------------------------------------------------------- limites noyau
  useEffect(() => {
    let cancelled = false;
    api.workspace.contextSettings().then(
      (value) => {
        if (!cancelled) {
          setSettings(value);
          setSettingsError(null);
        }
      },
      (reason: unknown) => {
        if (!cancelled) {
          setSettingsError(toAppError(reason));
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  // ------------------------------------------------------------- dimensions
  useEffect(() => {
    if (!hydrated) {
      return undefined;
    }
    const element = containerRef.current;
    if (!element) {
      return undefined;
    }
    const measure = () => {
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) {
        return;
      }
      const next = { width: Math.round(rect.width), height: Math.round(rect.height) };
      setBounds((current) =>
        current.width === next.width && current.height === next.height ? current : next,
      );
    };
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
    // La zone n'existe qu'une fois la disposition restaurée (et le projet ouvert).
  }, [hydrated]);

  useEffect(() => {
    setLayoutState((current) => {
      const next = clampLayout(current, bounds);
      return sameLayout(current, next) ? current : next;
    });
  }, [bounds]);

  // -------------------------------------------------------------- fichiers
  const loadFile = useCallback(
    async (path: string, silent = false): Promise<EditorTab | null> => {
      const scope = projectRef.current;
      if (!scope) {
        return null;
      }
      setFileBusy(true);
      try {
        const meta = await api.workspace.meta(scope, path);
        if (projectRef.current !== scope) {
          return null;
        }
        if (meta.is_binary) {
          if (!silent) {
            setNotice({ text: t("workspace.binary_file", { path }), tone: "warning" });
          }
          return null;
        }
        const limit = editorLimitRef.current;
        if (meta.size > limit) {
          if (!silent) {
            setNotice({
              text: t("workspace.file_too_large", { path }),
              tone: "warning",
            });
          }
          return null;
        }
        const read = await api.files.read(scope, path, limit);
        if (projectRef.current !== scope) {
          return null;
        }
        if (read.truncated) {
          if (!silent) {
            setNotice({
              text: t("workspace.file_too_large", { path }),
              tone: "warning",
            });
          }
          return null;
        }
        return createTab({
          path,
          content: read.content,
          sha256: meta.sha256,
          size: meta.size,
        });
      } catch (reason) {
        if (!silent && projectRef.current === scope) {
          setFileError(toAppError(reason));
        }
        return null;
      } finally {
        setFileBusy(false);
      }
    },
    [t],
  );

  // -------------------------------------------- chargement/restauration
  useEffect(() => {
    const scope = projectId;
    const token = ++scopeToken.current;
    setHydrated(false);
    setTabState({ tabs: [], activePath: null });
    setDiff(null);
    setDiffError(null);
    setFileError(null);
    setSaveError(null);
    setSaveConflict(null);
    setCloseTarget(null);
    setNotice(null);
    if (!scope) {
      setHydrated(true);
      return undefined;
    }
    let cancelled = false;
    const restore = async () => {
      let parsed = { ...DEFAULT_LAYOUT };
      try {
        parsed = parseLayout(await api.workspace.layoutGet());
      } catch (reason) {
        if (!cancelled && token === scopeToken.current) {
          setLayoutError(toAppError(reason));
        }
      }
      if (cancelled || token !== scopeToken.current) {
        return;
      }
      setLayoutState(clampLayout(parsed, boundsRef.current));
      const persisted = parsed.project_id === scope ? parsed.tabs.slice(0, MAX_RESTORED_TABS) : [];
      const loaded: RestoredTabFile[] = [];
      for (const path of persisted) {
        const tab = await loadFile(path, true);
        if (cancelled || token !== scopeToken.current) {
          return;
        }
        if (tab) {
          loaded.push({
            path: tab.path,
            content: tab.content,
            sha256: tab.sha256 ?? "",
            size: tab.size,
            readOnly: tab.readOnly,
          });
        }
      }
      if (cancelled || token !== scopeToken.current) {
        return;
      }
      const restored = restoreTabs(persisted, loaded, parsed.active_tab);
      setTabState(restored);
      setHydrated(true);
    };
    void restore();
    return () => {
      cancelled = true;
    };
    // `loadFile` est stable pour une locale donnée ; la portée est le projet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  // ------------------------------------------------------ persistance layout
  const pathsKey = tabState.tabs.map((tab) => tab.path).join("\n");
  useEffect(() => {
    if (!hydrated || !projectId) {
      return undefined;
    }
    const paths = pathsKey.length > 0 ? pathsKey.split("\n") : [];
    const payload = serializeLayout({
      ...layout,
      project_id: projectId,
      tabs: paths,
      active_tab: tabState.activePath,
    });
    const timer = window.setTimeout(() => {
      api.workspace.layoutSave(payload).then(
        () => setLayoutError(null),
        (reason: unknown) => setLayoutError(toAppError(reason)),
      );
    }, 500);
    return () => window.clearTimeout(timer);
  }, [hydrated, projectId, layout, pathsKey, tabState.activePath]);

  // -------------------------------------------------------------- disposition
  const updateLayout = useCallback((patch: Partial<WorkspaceLayout>) => {
    setLayoutState((current) => clampLayout({ ...current, ...patch }, boundsRef.current));
  }, []);

  const resetLayout = useCallback(() => {
    setLayoutState(
      clampLayout(
        {
          ...DEFAULT_LAYOUT,
          project_id: projectRef.current,
          tabs: tabsRef.current.map((tab) => tab.path),
          active_tab: activePathRef.current,
        },
        boundsRef.current,
      ),
    );
  }, []);

  const toggleExplorer = useCallback(() => {
    setLayoutState((current) =>
      clampLayout({ ...current, explorer_visible: !current.explorer_visible }, boundsRef.current),
    );
  }, []);

  const toggleBottom = useCallback(() => {
    setLayoutState((current) =>
      clampLayout({ ...current, bottom_open: !current.bottom_open }, boundsRef.current),
    );
  }, []);

  const setBottomTab = useCallback((tab: BottomTab) => {
    setLayoutState((current) =>
      clampLayout({ ...current, bottom_tab: normalizeBottomTab(tab) }, boundsRef.current),
    );
  }, []);

  // ------------------------------------------------------------------ onglets
  const openFile = useCallback(
    async (path: string) => {
      setDiff(null);
      setDiffError(null);
      const existing = tabsRef.current.find((tab) => tab.path === path);
      if (existing) {
        setSaveConflict((current) => (current && current.path !== path ? null : current));
        setTabState((state) => activateTabState(state.tabs, state.activePath, path));
        return;
      }
      const tab = await loadFile(path);
      if (!tab) {
        return;
      }
      setSaveConflict((current) => (current && current.path !== tab.path ? null : current));
      setTabState((current) => openTabState(current.tabs, current.activePath, tab));
    },
    [loadFile],
  );

  const activate = useCallback((path: string) => {
    setSaveConflict((current) => (current && current.path !== path ? null : current));
    setTabState((current) => activateTabState(current.tabs, current.activePath, path));
  }, []);

  const updateContent = useCallback((path: string, content: string) => {
    setTabState((current) => ({
      ...current,
      tabs: updateTabContent(current.tabs, path, content),
    }));
  }, []);

  const applyClose = useCallback((path: string) => {
    setTabState((current) => closeTabState(current.tabs, current.activePath, path));
    setDiff((current) => (current?.path === path ? null : current));
    setSaveConflict((current) => (current?.path === path ? null : current));
  }, []);

  const requestClose = useCallback(
    (path: string) => {
      const plan = planCloseTab(tabsRef.current, path);
      if (plan.requiresConfirmation) {
        setCloseTarget(path);
        return;
      }
      applyClose(path);
    },
    [applyClose],
  );

  const confirmClose = useCallback(() => {
    const path = closeTarget;
    setCloseTarget(null);
    if (path) {
      applyClose(path);
    }
  }, [applyClose, closeTarget]);

  const cancelClose = useCallback(() => setCloseTarget(null), []);

  // ------------------------------------------------------------- sauvegarde
  const save = useCallback(
    async (force = false, path: string | null = null) => {
      const scope = projectRef.current;
      const target = path ?? activePathRef.current;
      const current = tabsRef.current.find((tab) => tab.path === target);
      if (!scope || !current || current.readOnly) {
        return;
      }
      if (!force && !isTabDirty(current)) {
        return;
      }
      if (saveBusyRef.current) {
        return;
      }
      saveBusyRef.current = true;
      setSaveBusy(true);
      setSaveError(null);
      setSaveConflict(null);
      try {
        const outcome = await api.workspace.save(
          scope,
          current.path,
          current.content,
          force ? null : current.sha256,
        );
        if (projectRef.current !== scope) {
          return;
        }
        setTabState((state) => ({
          ...state,
          tabs: markTabSaved(state.tabs, current.path, {
            content: current.content,
            sha256: outcome.sha256,
            size: outcome.bytes,
          }),
        }));
        setNotice({
          text: t("workspace.saved", { path: current.path }),
          tone: "success",
        });
      } catch (reason) {
        if (projectRef.current !== scope) {
          return;
        }
        const error = toAppError(reason);
        if (error.code === "conflict") {
          setSaveConflict({ error, path: current.path });
        } else {
          setSaveError(error);
        }
      } finally {
        saveBusyRef.current = false;
        setSaveBusy(false);
      }
    },
    [t],
  );

  const reloadActive = useCallback(async () => {
    const current = tabsRef.current.find((tab) => tab.path === activePathRef.current);
    if (!current) {
      return;
    }
    const fresh = await loadFile(current.path);
    if (!fresh) {
      return;
    }
    setTabState((state) => ({
      ...state,
      tabs: reloadTab(state.tabs, current.path, {
        content: fresh.content,
        sha256: fresh.sha256 ?? "",
        size: fresh.size,
        readOnly: fresh.readOnly,
      }),
    }));
    setSaveConflict(null);
    setSaveError(null);
    setNotice({ text: t("workspace.reloaded", { path: current.path }), tone: "success" });
  }, [loadFile, t]);

  // ------------------------------------------------------------------- diff
  const showDiff = useCallback(async () => {
    const scope = projectRef.current;
    const current = tabsRef.current.find((tab) => tab.path === activePathRef.current);
    if (!scope || !current || !isTabDirty(current)) {
      return;
    }
    setDiffBusy(true);
    setDiffError(null);
    try {
      const preview = await api.workspace.diffPrepare(scope, current.path, current.content);
      if (projectRef.current !== scope) {
        return;
      }
      setDiff(preview);
    } catch (reason) {
      if (projectRef.current === scope) {
        setDiffError(toAppError(reason));
      }
    } finally {
      setDiffBusy(false);
    }
  }, []);

  const closeDiff = useCallback(() => {
    setDiff(null);
    setDiffError(null);
  }, []);

  const acceptDiff = useCallback(async () => {
    const preview = diffRef.current;
    const scope = projectRef.current;
    if (!preview || !scope) {
      return;
    }
    setDiffBusy(true);
    setDiffError(null);
    try {
      const outcome = await api.workspace.diffAccept(preview.id);
      if (projectRef.current !== scope) {
        return;
      }
      // Relit le disque pour repartir d'un buffer et d'une empreinte exacts.
      let content: string | null = null;
      let readOnly = false;
      try {
        const read = await api.files.read(scope, preview.path, editorLimitRef.current);
        if (!read.truncated) {
          content = read.content;
        } else {
          readOnly = true;
        }
      } catch {
        content = null;
      }
      setTabState((state) => ({
        ...state,
        tabs:
          content !== null
            ? reloadTab(state.tabs, preview.path, {
                content,
                sha256: outcome.sha256,
                size: outcome.bytes,
              })
            : state.tabs.map((tab) =>
                tab.path === preview.path
                  ? {
                      ...tab,
                      savedContent: tab.content,
                      sha256: outcome.sha256,
                      size: outcome.bytes,
                      readOnly: readOnly || tab.readOnly,
                    }
                  : tab,
              ),
      }));
      setDiff(null);
      setNotice({
        text: t("workspace.diff_accepted", { path: preview.path }),
        tone: "success",
      });
      void api.workspace.diffPending().then(setPendingDiffs, () => undefined);
    } catch (reason) {
      if (projectRef.current === scope) {
        setDiffError(toAppError(reason));
      }
    } finally {
      setDiffBusy(false);
    }
  }, [t]);

  const rejectDiff = useCallback(async () => {
    const preview = diffRef.current;
    if (!preview) {
      return;
    }
    setDiffBusy(true);
    setDiffError(null);
    try {
      await api.workspace.diffReject(preview.id);
      setDiff(null);
      setNotice({
        text: t("workspace.diff_rejected", { path: preview.path }),
        tone: "info",
      });
      void api.workspace.diffPending().then(setPendingDiffs, () => undefined);
    } catch (reason) {
      setDiffError(toAppError(reason));
    } finally {
      setDiffBusy(false);
    }
  }, [t]);

  // -------------------------------------------------- diffs en attente (git)
  const refreshPendingDiffs = useCallback(async () => {
    setPendingDiffBusy(true);
    setPendingDiffError(null);
    try {
      setPendingDiffs(await api.workspace.diffPending());
    } catch (reason) {
      setPendingDiffError(toAppError(reason));
    } finally {
      setPendingDiffBusy(false);
    }
  }, []);

  const refreshTabBaseline = useCallback(async (path: string) => {
    const scope = projectRef.current;
    if (!scope) {
      return;
    }
    try {
      const [meta, read] = await Promise.all([
        api.workspace.meta(scope, path),
        api.files.read(scope, path, editorLimitRef.current),
      ]);
      if (projectRef.current !== scope) {
        return;
      }
      setTabState((state) => ({
        ...state,
        tabs: state.tabs.map((tab) => {
          if (tab.path !== path) {
            return tab;
          }
          if (read.truncated) {
            // Trop gros pour être relu : on ne remplace jamais le buffer.
            return {
              ...tab,
              sha256: meta.sha256,
              size: meta.size,
              readOnly: true,
            };
          }
          if (!isTabDirty(tab)) {
            // Onglet propre : on affiche la nouvelle version disque.
            return {
              ...tab,
              content: read.content,
              savedContent: read.content,
              sha256: meta.sha256,
              size: meta.size,
            };
          }
          // Buffer modifié : on rafraîchit seulement la référence disque.
          return {
            ...tab,
            savedContent: read.content,
            sha256: meta.sha256,
            size: meta.size,
            readOnly: tab.readOnly,
          };
        }),
      }));
    } catch {
      // Fichier supprimé entre-temps : le buffer et son empreinte restent.
    }
  }, []);

  const acceptPendingDiff = useCallback(
    async (id: string) => {
      const target = pendingDiffs.find((entry) => entry.id === id) ?? null;
      setPendingDiffBusy(true);
      setPendingDiffError(null);
      try {
        await api.workspace.diffAccept(id);
        await refreshPendingDiffs();
        if (target) {
          await refreshTabBaseline(target.path);
          setNotice({
            text: t("workspace.pending_accepted", { path: target.path }),
            tone: "success",
          });
        }
      } catch (reason) {
        setPendingDiffError(toAppError(reason));
      } finally {
        setPendingDiffBusy(false);
      }
    },
    [pendingDiffs, refreshPendingDiffs, refreshTabBaseline, t],
  );

  const rejectPendingDiff = useCallback(
    async (id: string) => {
      setPendingDiffBusy(true);
      setPendingDiffError(null);
      try {
        await api.workspace.diffReject(id);
        await refreshPendingDiffs();
        setNotice({ text: t("workspace.pending_rejected"), tone: "info" });
      } catch (reason) {
        setPendingDiffError(toAppError(reason));
      } finally {
        setPendingDiffBusy(false);
      }
    },
    [refreshPendingDiffs, t],
  );

  const dismissFileError = useCallback(() => setFileError(null), []);
  const dismissSaveError = useCallback(() => setSaveError(null), []);
  const dismissDiffError = useCallback(() => setDiffError(null), []);
  const dismissNotice = useCallback(() => setNotice(null), []);
  const dismissLayoutError = useCallback(() => setLayoutError(null), []);

  return {
    layout,
    bounds,
    hydrated,
    settings,
    settingsError,
    tabs: tabState.tabs,
    activePath: tabState.activePath,
    activeTab,
    dirty,
    fileError,
    saveError,
    saveConflict,
    diff,
    diffError,
    diffBusy,
    saveBusy,
    fileBusy,
    layoutError,
    pendingDiffs,
    pendingDiffError,
    pendingDiffBusy,
    closeTarget,
    notice,
    containerRef,
    updateLayout,
    resetLayout,
    toggleExplorer,
    toggleBottom,
    setBottomTab,
    openFile,
    activate,
    updateContent,
    save,
    reloadActive,
    requestClose,
    confirmClose,
    cancelClose,
    showDiff,
    closeDiff,
    acceptDiff,
    rejectDiff,
    refreshPendingDiffs,
    acceptPendingDiff,
    rejectPendingDiff,
    dismissFileError,
    dismissSaveError,
    dismissDiffError,
    dismissNotice,
    dismissLayoutError,
  };
}
