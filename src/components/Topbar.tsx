import { FolderOpen, Languages } from "lucide-react";
import { useApp } from "../lib/store";

export function Topbar() {
  const { t, section, project, projects, projectsLoading, selectProject, navigate, locale, setLocale } =
    useApp();
  return (
    <header className="topbar">
      <div className="breadcrumb">
        <span className="breadcrumb-root">{t("app.name")}</span>
        <span aria-hidden="true">/</span>
        <span>{t(`nav.${section}`)}</span>
      </div>
      <div className="topbar-actions">
        <label className="project-switch">
          <span className="project-switch-label">{t("projects.current")}</span>
          <select
            aria-label={t("projects.current")}
            value={project?.id ?? ""}
            disabled={projectsLoading}
            onChange={(event) => void selectProject(event.target.value || null)}
          >
            <option value="">{t("overview.no_project")}</option>
            {projects.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.name}
              </option>
            ))}
          </select>
        </label>
        {projects.length === 0 ? (
          <button type="button" className="ghost small" onClick={() => navigate("projects")}>
            <FolderOpen size={14} aria-hidden="true" />
            {t("projects.add")}
          </button>
        ) : null}
        <div className="lang-switch" role="group" aria-label={t("common.language")}>
          <Languages size={15} aria-hidden="true" />
          <button
            type="button"
            className={locale === "fr" ? "lang-button active" : "lang-button"}
            aria-pressed={locale === "fr"}
            aria-label={t("common.french")}
            onClick={() => void setLocale("fr")}
          >
            FR
          </button>
          <button
            type="button"
            className={locale === "en" ? "lang-button active" : "lang-button"}
            aria-pressed={locale === "en"}
            aria-label={t("common.english")}
            onClick={() => void setLocale("en")}
          >
            EN
          </button>
        </div>
      </div>
    </header>
  );
}
