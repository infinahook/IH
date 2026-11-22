import {
  Activity,
  Bot,
  Brain,
  BrainCircuit,
  Clock,
  FileCode2,
  FolderOpen,
  GitBranch,
  History,
  ListChecks,
  Plug,
  Settings,
  ShieldCheck,
  Stethoscope,
  Terminal,
  WandSparkles,
  type LucideIcon,
} from "lucide-react";
import { SECTIONS, type SectionId } from "../lib/navigation";
import { useApp } from "../lib/store";

const ICONS: Record<SectionId, LucideIcon> = {
  overview: Activity,
  projects: FolderOpen,
  agents: Bot,
  models: BrainCircuit,
  skills: WandSparkles,
  plugins: Plug,
  terminal: Terminal,
  git: GitBranch,
  editor: FileCode2,
  tasks: ListChecks,
  memory: Brain,
  security: ShieldCheck,
  automation: Clock,
  history: History,
  settings: Settings,
  diagnostics: Stethoscope,
};

export function Sidebar() {
  const { t, section, navigate, health } = useApp();
  const online = health?.core_online === true;
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark" aria-hidden="true">
          IH
        </div>
        <div>
          <strong>{t("app.name")}</strong>
          <small>{t("app.tagline")}</small>
        </div>
      </div>
      <nav className="nav" aria-label={t("nav.label")}>
        {SECTIONS.map((entry) => {
          const Icon = ICONS[entry.id];
          const active = entry.id === section;
          return (
            <button
              key={entry.id}
              type="button"
              className={active ? "nav-item active" : "nav-item"}
              aria-current={active ? "page" : undefined}
              onClick={() => navigate(entry.id)}
            >
              <Icon size={17} aria-hidden="true" />
              <span>{t(entry.labelKey)}</span>
            </button>
          );
        })}
      </nav>
      <div className="sidebar-bottom">
        <span className={online ? "online-dot" : "online-dot offline"} aria-hidden="true" />
        <span>{online ? t("status.online") : t("status.offline")}</span>
        {health?.version ? <span className="sidebar-version">v{health.version}</span> : null}
      </div>
    </aside>
  );
}
