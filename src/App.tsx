import { useEffect, type ComponentType } from "react";
import { RefreshCw } from "lucide-react";
import { ErrorBanner } from "./components/ErrorBanner";
import { LoadingRow } from "./components/EmptyState";
import { Sidebar } from "./components/Sidebar";
import { Topbar } from "./components/Topbar";
import { SECTIONS, type SectionId } from "./lib/navigation";
import { AppProvider, useApp } from "./lib/store";
import { OverviewPage } from "./pages/OverviewPage";
import { ProjectsPage } from "./pages/ProjectsPage";
import { AgentsPage } from "./pages/AgentsPage";
import { ModelsPage } from "./pages/ModelsPage";
import { SkillsPage } from "./pages/SkillsPage";
import { PluginsPage } from "./pages/PluginsPage";
import { TerminalPage } from "./pages/TerminalPage";
import { GitPage } from "./pages/GitPage";
import { EditorPage } from "./pages/EditorPage";
import { TasksPage } from "./pages/TasksPage";
import { MemoryPage } from "./pages/MemoryPage";
import { SecurityPage } from "./pages/SecurityPage";
import { AutomationPage } from "./pages/AutomationPage";
import { HistoryPage } from "./pages/HistoryPage";
import { SettingsPage } from "./pages/SettingsPage";
import { DiagnosticsPage } from "./pages/DiagnosticsPage";

const PAGES: Record<SectionId, ComponentType> = {
  overview: OverviewPage,
  projects: ProjectsPage,
  agents: AgentsPage,
  models: ModelsPage,
  skills: SkillsPage,
  plugins: PluginsPage,
  terminal: TerminalPage,
  git: GitPage,
  editor: EditorPage,
  tasks: TasksPage,
  memory: MemoryPage,
  security: SecurityPage,
  automation: AutomationPage,
  history: HistoryPage,
  settings: SettingsPage,
  diagnostics: DiagnosticsPage,
};

function Shell() {
  const {
    t,
    section,
    booting,
    bootError,
    globalError,
    setGlobalError,
    reloadCore,
    health,
  } = useApp();
  const Page = PAGES[section];
  const sectionLabel = SECTIONS.find((entry) => entry.id === section)?.labelKey ?? "nav.overview";
  const offline = !health || !health.core_online;

  useEffect(() => {
    document.title = `IH — ${t(sectionLabel)}`;
    window.scrollTo({ top: 0, behavior: "auto" });
  }, [section, sectionLabel, t]);

  if (booting) {
    return (
      <div className="boot-screen">
        <div className="brand-mark" aria-hidden="true">
          IH
        </div>
        <p className="boot-title">{t("app.name")}</p>
        <LoadingRow label={t("app.loading")} />
      </div>
    );
  }

  return (
    <div className="shell">
      <a className="skip-link" href="#main-content">
        {t("common.skip_to_content")}
      </a>
      <Sidebar />
      <main className="main" id="main-content" tabIndex={-1}>
        <Topbar />
        <section className="content">
          {bootError ? (
            <ErrorBanner error={bootError} onRetry={() => void reloadCore()} />
          ) : null}
          {globalError ? (
            <ErrorBanner error={globalError} onDismiss={() => setGlobalError(null)} />
          ) : null}
          {offline ? (
            <div className="offline-card" role="alert">
              <h1>{t("status.offline")}</h1>
              <p className="muted">{t("overview.core_offline_hint")}</p>
              <button type="button" className="primary" onClick={() => void reloadCore()}>
                <RefreshCw size={15} aria-hidden="true" />
                {t("common.retry")}
              </button>
            </div>
          ) : (
            <Page />
          )}
        </section>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}
