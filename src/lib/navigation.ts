export type SectionId =
  | "chat"
  | "overview"
  | "projects"
  | "agents"
  | "models"
  | "providers"
  | "skills"
  | "plugins"
  | "mcp"
  | "terminal"
  | "git"
  | "editor"
  | "tasks"
  | "memory"
  | "security"
  | "automation"
  | "history"
  | "settings"
  | "diagnostics";

export interface SectionDefinition {
  id: SectionId;
  labelKey: string;
}

/** Les 19 sections réelles de l'application, dans l'ordre de la barre latérale. */
export const SECTIONS: readonly SectionDefinition[] = [
  { id: "chat", labelKey: "nav.chat" },
  { id: "overview", labelKey: "nav.overview" },
  { id: "projects", labelKey: "nav.projects" },
  { id: "agents", labelKey: "nav.agents" },
  { id: "models", labelKey: "nav.models" },
  { id: "providers", labelKey: "nav.providers" },
  { id: "skills", labelKey: "nav.skills" },
  { id: "plugins", labelKey: "nav.plugins" },
  { id: "mcp", labelKey: "nav.mcp" },
  { id: "terminal", labelKey: "nav.terminal" },
  { id: "git", labelKey: "nav.git" },
  { id: "editor", labelKey: "nav.editor" },
  { id: "tasks", labelKey: "nav.tasks" },
  { id: "memory", labelKey: "nav.memory" },
  { id: "security", labelKey: "nav.security" },
  { id: "automation", labelKey: "nav.automation" },
  { id: "history", labelKey: "nav.history" },
  { id: "settings", labelKey: "nav.settings" },
  { id: "diagnostics", labelKey: "nav.diagnostics" },
];

export const DEFAULT_SECTION: SectionId = "chat";
