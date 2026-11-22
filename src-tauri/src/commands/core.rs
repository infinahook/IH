//! Commandes noyau : santé, configuration, projets, fichiers, diagnostic,
//! approbations.

use super::project_path;
use crate::state::AppState;
use ih_core::config::AppConfig;
use ih_core::events::TaskState;
use ih_core::platform::PlatformInfo;
use ih_core::{IhError, IhResult, ProjectId};
use ih_security::approval::{ApprovalDecision, ApprovalRequest};
use ih_security::risk::{classify_command, CommandAssessment};
use ih_security::secrets::SecretStoreStatus;
use ih_workspace::fs::{FileContent, FileNode, ProjectFs, SearchHit};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Serialize)]
pub struct Health {
    pub app: &'static str,
    pub version: &'static str,
    pub platform: String,
    pub locale: String,
    pub core_online: bool,
}

#[derive(Debug, Serialize)]
pub struct PathsInfo {
    pub root: String,
    pub config_file: String,
    pub database: String,
    pub logs: String,
    pub skills: String,
    pub plugins: String,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticsReport {
    pub generated_at: String,
    pub platform: PlatformInfo,
    pub app_version: String,
    pub locale: String,
    pub projects: usize,
    pub threads: usize,
    pub tasks: usize,
    pub memory_entries: usize,
    pub installed_skills: usize,
    pub installed_plugins: usize,
    pub database: String,
    pub log_tail: Vec<String>,
}

#[tauri::command]
pub fn app_health(state: State<'_, AppState>) -> IhResult<Health> {
    Ok(Health {
        app: "IH",
        version: ih_core::APP_VERSION,
        platform: state.platform.os_name.clone(),
        locale: state.config_snapshot()?.locale,
        core_online: true,
    })
}

#[tauri::command]
pub fn platform_info(state: State<'_, AppState>) -> PlatformInfo {
    state.platform.clone()
}

#[tauri::command]
pub fn config_get(state: State<'_, AppState>) -> IhResult<AppConfig> {
    state.config_snapshot()
}

#[tauri::command]
pub fn config_update(state: State<'_, AppState>, patch: serde_json::Value) -> IhResult<AppConfig> {
    let patch: AppConfig = serde_json::from_value(patch)
        .map_err(|e| IhError::Invalid(format!("configuration invalide : {e}")))?;
    state.update_config(|current| *current = patch)
}

#[tauri::command]
pub fn locale_set(state: State<'_, AppState>, locale: String) -> IhResult<AppConfig> {
    state.set_locale(&locale)
}

#[tauri::command]
pub fn paths_info(state: State<'_, AppState>) -> PathsInfo {
    PathsInfo {
        root: state.paths.root().display().to_string(),
        config_file: state.paths.config_file().display().to_string(),
        database: state.paths.db_path().display().to_string(),
        logs: state.paths.log_dir().display().to_string(),
        skills: state.paths.skills_dir().display().to_string(),
        plugins: state.paths.plugins_dir().display().to_string(),
    }
}

#[tauri::command]
pub fn secret_store_status(state: State<'_, AppState>) -> SecretStoreStatus {
    state.secret_status()
}

#[tauri::command]
pub fn classify_command_preview(command: String) -> CommandAssessment {
    classify_command(&command)
}

#[tauri::command]
pub fn approvals_pending(state: State<'_, AppState>) -> IhResult<Vec<ApprovalRequest>> {
    state.approvals.list_pending()
}

#[tauri::command]
pub fn approval_resolve(
    state: State<'_, AppState>,
    id: String,
    decision: String,
) -> IhResult<ApprovalRequest> {
    let decision = ApprovalDecision::parse(&decision)?;
    state.approvals.resolve(&id, decision)
}

// ---------------------------------------------------------------- projets

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_git: bool,
    pub last_opened_at: String,
}

#[tauri::command]
pub fn project_add(state: State<'_, AppState>, path: String) -> IhResult<ProjectView> {
    let path_buf = PathBuf::from(&path);
    if !path_buf.is_dir() {
        return Err(IhError::Invalid(format!("dossier inexistant : {path}")));
    }
    let canonical = std::fs::canonicalize(&path_buf)?;
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| canonical.display().to_string());

    let project = ih_store::Project {
        id: ProjectId::new(),
        path: canonical.display().to_string(),
        name,
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
        trusted: true,
    };
    state.store.upsert_project(&project)?;
    state.update_config(|config| config.remember_project(&project.path))?;
    Ok(ProjectView {
        id: project.id.into_string(),
        name: project.name,
        path: project.path,
        is_git: ih_workspace::git::GitRepo::discover(&canonical).is_ok(),
        last_opened_at: project.last_opened_at.to_rfc3339(),
    })
}

#[tauri::command]
pub fn project_list(state: State<'_, AppState>) -> IhResult<Vec<ProjectView>> {
    let projects = state.store.list_projects()?;
    Ok(projects
        .into_iter()
        .map(|p| ProjectView {
            is_git: Path::new(&p.path).join(".git").exists(),
            id: p.id.into_string(),
            name: p.name,
            path: p.path,
            last_opened_at: p.last_opened_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub fn project_remove(state: State<'_, AppState>, id: String) -> IhResult<()> {
    let project_id = ProjectId::parse(id)?;
    state.store.remove_project(&project_id)
}

// ------------------------------------------------------------------ fichiers

#[tauri::command]
pub fn file_tree(
    state: State<'_, AppState>,
    project_id: String,
    depth: usize,
) -> IhResult<Vec<FileNode>> {
    let root = project_path(&state, &project_id)?;
    let fs = ProjectFs::new(&root)?;
    fs.list_tree(depth.clamp(1, 8), 5_000)
}

#[tauri::command]
pub fn file_read(
    state: State<'_, AppState>,
    project_id: String,
    path: String,
    max_bytes: Option<usize>,
) -> IhResult<FileContent> {
    let root = project_path(&state, &project_id)?;
    let fs = ProjectFs::new(&root)?;
    fs.read_text(&path, max_bytes.unwrap_or(1_000_000))
}

#[tauri::command]
pub fn file_write(
    state: State<'_, AppState>,
    project_id: String,
    path: String,
    content: String,
) -> IhResult<u64> {
    let root = project_path(&state, &project_id)?;
    let fs = ProjectFs::new(&root)?;
    fs.write_text(&path, &content)
}

#[tauri::command]
pub fn file_search(
    state: State<'_, AppState>,
    project_id: String,
    query: String,
    limit: Option<usize>,
) -> IhResult<Vec<SearchHit>> {
    let root = project_path(&state, &project_id)?;
    let fs = ProjectFs::new(&root)?;
    fs.search(&query, limit.unwrap_or(100).clamp(1, 500))
}

// -------------------------------------------------------------- diagnostic

#[tauri::command]
pub fn log_tail(state: State<'_, AppState>, lines: Option<usize>) -> IhResult<Vec<String>> {
    read_log_tail(&state.paths.log_dir(), lines.unwrap_or(200).clamp(1, 2_000))
}

#[tauri::command]
pub fn diagnostics_report(state: State<'_, AppState>) -> IhResult<DiagnosticsReport> {
    let config = state.config_snapshot()?;
    let projects = state.store.list_projects()?;
    let mut threads = 0;
    let mut tasks = 0;
    for project in &projects {
        threads += state.store.list_threads(&project.id)?.len();
        tasks += state.store.list_tasks(&project.id)?.len();
    }
    Ok(DiagnosticsReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        platform: state.platform.clone(),
        app_version: ih_core::APP_VERSION.to_string(),
        locale: config.locale,
        projects: projects.len(),
        threads,
        tasks,
        memory_entries: state.store.list_memory(None)?.len(),
        installed_skills: state.store.list_extensions("skill")?.len(),
        installed_plugins: state.store.list_extensions("plugin")?.len(),
        database: state.paths.db_path().display().to_string(),
        log_tail: read_log_tail(&state.paths.log_dir(), 100).unwrap_or_default(),
    })
}

fn read_log_tail(dir: &Path, lines: usize) -> IhResult<Vec<String>> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                newest = Some((modified, path));
            }
        }
    }
    let Some((_, path)) = newest else {
        return Ok(Vec::new());
    };
    let content = std::fs::read_to_string(path)?;
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(all[start..].iter().map(|s| s.to_string()).collect())
}

/// Utilitaire partagé par les commandes qui manipulent des états de tâche.
pub fn parse_task_state(value: &str) -> IhResult<TaskState> {
    match value {
        "pending" => Ok(TaskState::Pending),
        "ready" => Ok(TaskState::Ready),
        "running" => Ok(TaskState::Running),
        "blocked" => Ok(TaskState::Blocked),
        "completed" => Ok(TaskState::Completed),
        "failed" => Ok(TaskState::Failed),
        "cancelled" => Ok(TaskState::Cancelled),
        other => Err(IhError::Invalid(format!("état inconnu : {other}"))),
    }
}

/// Ouvre une URL externe (https/http uniquement) via `xdg-open`.
#[tauri::command]
pub fn open_external(url: String) -> IhResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(IhError::Denied(
            "seules les URL http(s) peuvent être ouvertes".into(),
        ));
    }
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map_err(|e| IhError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}

/// Ouvre un dossier ou fichier local (dossier de journaux, projet).
#[tauri::command]
pub fn open_path(path: String) -> IhResult<()> {
    let target = PathBuf::from(&path);
    if !target.exists() {
        return Err(IhError::NotFound(path));
    }
    std::process::Command::new("xdg-open")
        .arg(target)
        .spawn()
        .map_err(|e| IhError::Io(std::io::Error::other(e.to_string())))?;
    Ok(())
}
