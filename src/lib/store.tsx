import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  api,
  toAppError,
  type AppConfig,
  type AppError,
  type Health,
  type PlatformInfo,
  type ProjectView,
  type SecretStoreStatus,
} from "./api";
import { useTauriEvent } from "./events";
import { createTranslator, normalizeLocale, type Locale, type TFunction } from "./i18n";
import { DEFAULT_SECTION, type SectionId } from "./navigation";

export interface PendingToolCall {
  approvalId: string;
  serverId: string;
  tool: string;
  args: Record<string, unknown>;
}

export interface AppStoreValue {
  config: AppConfig | null;
  health: Health | null;
  platform: PlatformInfo | null;
  projects: ProjectView[];
  project: ProjectView | null;
  projectsLoading: boolean;
  booting: boolean;
  bootError: AppError | null;
  globalError: AppError | null;
  locale: Locale;
  t: TFunction;
  theme: string;
  section: SectionId;
  secretStatus: SecretStoreStatus | null;
  pendingToolCall: PendingToolCall | null;
  updateAvailable: string | null;
  navigate: (section: SectionId) => void;
  reloadCore: () => Promise<void>;
  refreshConfig: () => Promise<AppConfig | null>;
  saveConfig: (next: AppConfig) => Promise<AppConfig | null>;
  setLocale: (locale: Locale) => Promise<void>;
  refreshProjects: () => Promise<void>;
  addProject: (path: string) => Promise<ProjectView>;
  removeProject: (projectId: string) => Promise<void>;
  selectProject: (projectId: string | null) => Promise<void>;
  setPendingToolCall: (value: PendingToolCall | null) => void;
  clearUpdateAvailable: () => void;
  setGlobalError: (error: AppError | null) => void;
}

const AppStoreContext = createContext<AppStoreValue | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [health, setHealth] = useState<Health | null>(null);
  const [platform, setPlatform] = useState<PlatformInfo | null>(null);
  const [projects, setProjects] = useState<ProjectView[]>([]);
  const [project, setProject] = useState<ProjectView | null>(null);
  const [projectsLoading, setProjectsLoading] = useState(true);
  const [booting, setBooting] = useState(true);
  const [bootError, setBootError] = useState<AppError | null>(null);
  const [globalError, setGlobalError] = useState<AppError | null>(null);
  const [secretStatus, setSecretStatus] = useState<SecretStoreStatus | null>(null);
  const [pendingToolCall, setPendingToolCall] = useState<PendingToolCall | null>(null);
  const [updateAvailable, setUpdateAvailable] = useState<string | null>(null);
  const [section, setSection] = useState<SectionId>(DEFAULT_SECTION);

  const locale = normalizeLocale(config?.locale);
  const t = useMemo(() => createTranslator(locale), [locale]);
  const theme = config?.theme ?? "dark";

  const reloadCore = useCallback(async () => {
    setBooting(true);
    setProjectsLoading(true);
    const results = await Promise.allSettled([
      api.configGet(),
      api.health(),
      api.platformInfo(),
      api.projects.list(),
      api.secretStoreStatus(),
    ]);
    const [configResult, healthResult, platformResult, projectsResult, secretResult] = results;
    let loadedConfig: AppConfig | null = null;
    let loadedProjects: ProjectView[] | null = null;

    if (configResult.status === "fulfilled") {
      loadedConfig = configResult.value;
      setConfig(loadedConfig);
    }
    if (healthResult.status === "fulfilled") {
      setHealth(healthResult.value);
    } else {
      setHealth(null);
    }
    if (platformResult.status === "fulfilled") {
      setPlatform(platformResult.value);
    }
    if (projectsResult.status === "fulfilled") {
      loadedProjects = projectsResult.value;
      setProjects(loadedProjects);
    }
    if (secretResult.status === "fulfilled") {
      setSecretStatus(secretResult.value);
    }

    const failures = results.flatMap((result) =>
      result.status === "rejected" ? [toAppError(result.reason)] : [],
    );
    setBootError(failures[0] ?? null);

    if (loadedProjects) {
      const currentProjects = loadedProjects;
      const lastProject = loadedConfig?.last_project ?? null;
      setProject((current) => {
        if (current) {
          const refreshed = currentProjects.find((entry) => entry.id === current.id);
          if (refreshed) {
            return refreshed;
          }
        }
        if (!lastProject) {
          return null;
        }
        return currentProjects.find((entry) => entry.path === lastProject) ?? null;
      });
    }

    setProjectsLoading(false);
    setBooting(false);
  }, []);

  useEffect(() => {
    void reloadCore();
  }, [reloadCore]);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: light)");
    const apply = () => {
      const resolved =
        theme === "system" ? (media.matches ? "light" : "dark") : theme === "light" ? "light" : "dark";
      document.documentElement.dataset.theme = resolved;
    };
    apply();
    if (theme !== "system") {
      return undefined;
    }
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  const updateEventError = useTauriEvent<{ version?: string }>(
    "ih://update-available",
    (payload) => {
      if (typeof payload.version === "string" && payload.version.length > 0) {
        setUpdateAvailable(payload.version);
      }
    },
  );

  useEffect(() => {
    if (updateEventError) {
      setGlobalError(updateEventError);
    }
  }, [updateEventError]);

  const navigate = useCallback((next: SectionId) => {
    setSection(next);
  }, []);

  const refreshConfig = useCallback(async (): Promise<AppConfig | null> => {
    try {
      const fresh = await api.configGet();
      setConfig(fresh);
      return fresh;
    } catch (reason) {
      setGlobalError(toAppError(reason));
      return null;
    }
  }, []);

  const saveConfig = useCallback(async (next: AppConfig): Promise<AppConfig | null> => {
    try {
      const saved = await api.configUpdate(next);
      setConfig(saved);
      setGlobalError(null);
      return saved;
    } catch (reason) {
      setGlobalError(toAppError(reason));
      return null;
    }
  }, []);

  const setLocale = useCallback(async (next: Locale): Promise<void> => {
    try {
      const saved = await api.localeSet(next);
      setConfig(saved);
      setGlobalError(null);
    } catch (reason) {
      setGlobalError(toAppError(reason));
    }
  }, []);

  const refreshProjects = useCallback(async (): Promise<void> => {
    setProjectsLoading(true);
    try {
      const list = await api.projects.list();
      setProjects(list);
      setProject((current) =>
        current ? list.find((entry) => entry.id === current.id) ?? null : null,
      );
      setProjectsLoading(false);
    } catch (reason) {
      setGlobalError(toAppError(reason));
      setProjectsLoading(false);
    }
  }, []);

  const addProject = useCallback(async (path: string): Promise<ProjectView> => {
    const added = await api.projects.add(path);
    const [list, fresh] = await Promise.all([api.projects.list(), api.configGet()]);
    setProjects(list);
    setConfig(fresh);
    setProject(list.find((entry) => entry.id === added.id) ?? added);
    return added;
  }, []);

  const removeProject = useCallback(
    async (projectId: string): Promise<void> => {
      await api.projects.remove(projectId);
      const remaining = await api.projects.list();
      setProjects(remaining);
      if (project?.id === projectId) {
        setProject(null);
      }
      try {
        const fresh = await api.configGet();
        const removed = projects.find((entry) => entry.id === projectId);
        if (removed && fresh.last_project === removed.path) {
          const updated = await api.configUpdate({
            ...fresh,
            last_project: null,
            recent_projects: fresh.recent_projects.filter((entry) => entry !== removed.path),
          });
          setConfig(updated);
        } else {
          setConfig(fresh);
        }
      } catch (reason) {
        setGlobalError(toAppError(reason));
      }
    },
    [project?.id, projects],
  );

  const selectProject = useCallback(
    async (projectId: string | null): Promise<void> => {
      const target = projectId ? projects.find((entry) => entry.id === projectId) ?? null : null;
      setProject(target);
      if (!config) {
        return;
      }
      try {
        // Relit la configuration avant écriture pour ne pas écraser une
        // modification faite par une autre page (fournisseur, locale…).
        const fresh = await api.configGet();
        const recent = target
          ? [target.path, ...fresh.recent_projects.filter((entry) => entry !== target.path)].slice(0, 12)
          : fresh.recent_projects;
        const saved = await api.configUpdate({
          ...fresh,
          last_project: target?.path ?? null,
          recent_projects: recent,
        });
        setConfig(saved);
      } catch (reason) {
        setGlobalError(toAppError(reason));
      }
    },
    [config, projects],
  );

  const clearUpdateAvailable = useCallback(() => setUpdateAvailable(null), []);

  const value: AppStoreValue = {
    config,
    health,
    platform,
    projects,
    project,
    projectsLoading,
    booting,
    bootError,
    globalError,
    locale,
    t,
    theme,
    section,
    secretStatus,
    pendingToolCall,
    updateAvailable,
    navigate,
    reloadCore,
    refreshConfig,
    saveConfig,
    setLocale,
    refreshProjects,
    addProject,
    removeProject,
    selectProject,
    setPendingToolCall,
    clearUpdateAvailable,
    setGlobalError,
  };

  return <AppStoreContext.Provider value={value}>{children}</AppStoreContext.Provider>;
}

export function useApp(): AppStoreValue {
  const value = useContext(AppStoreContext);
  if (!value) {
    throw new Error("useApp doit être utilisé dans <AppProvider>");
  }
  return value;
}
