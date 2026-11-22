//! Commandes du workspace : fichiers, contexte projet, diff et disposition.
//!
//! Toutes les lectures/écritures passent par `ih_workspace::fs::ProjectFs`
//! (prison de chemins) et tout le contexte est assemblé par `ih-context` :
//! l'interface ne lit jamais un fichier pour le compte du modèle.

use super::project_path;
use crate::state::AppState;
use ih_context::{ContextOptions, MentionCandidate, PreparedContext};
use ih_core::{IhError, IhResult};
use ih_workspace::diff::{diff_lines, LineDiff};
use ih_workspace::fs::{FileMeta, ProjectFs, WriteOutcome};
use serde::Serialize;
use tauri::State;

/// Taille maximale d'une disposition sauvegardée (JSON).
const MAX_LAYOUT_BYTES: usize = 64 * 1024;
/// Clé de préférence de la disposition du workspace.
const LAYOUT_KEY: &str = "workspace.layout";

/// Limites et exclusions du contexte, exposées à l'interface pour l'affichage.
#[derive(Debug, Serialize)]
pub struct ContextSettings {
    pub max_files: usize,
    pub max_bytes_per_file: usize,
    pub max_total_bytes: usize,
    pub max_editor_bytes: usize,
    pub exclusions: Vec<String>,
}

#[tauri::command]
pub fn context_settings(state: State<'_, AppState>) -> IhResult<ContextSettings> {
    let config = state.config_snapshot()?;
    let mut exclusions = ih_context::default_exclusions();
    exclusions.extend(config.workspace.exclusions.clone());
    Ok(ContextSettings {
        max_files: config.workspace.max_context_files,
        max_bytes_per_file: config.workspace.max_context_bytes_per_file,
        max_total_bytes: config.workspace.max_context_bytes,
        max_editor_bytes: config.workspace.max_editor_bytes,
        exclusions,
    })
}

fn options(state: &AppState, allow_sensitive: bool) -> IhResult<ContextOptions> {
    let config = state.config_snapshot()?;
    Ok(ContextOptions {
        max_files: config.workspace.max_context_files,
        max_bytes_per_file: config.workspace.max_context_bytes_per_file,
        max_total_bytes: config.workspace.max_context_bytes,
        allow_sensitive,
        extra_exclusions: config.workspace.exclusions.clone(),
    })
}

#[tauri::command]
pub fn workspace_meta(
    state: State<'_, AppState>,
    project_id: String,
    path: String,
) -> IhResult<FileMeta> {
    let root = project_path(&state, &project_id)?;
    ProjectFs::new(&root)?.metadata(&path)
}

#[tauri::command]
pub fn workspace_save(
    state: State<'_, AppState>,
    project_id: String,
    path: String,
    content: String,
    expected_sha256: Option<String>,
) -> IhResult<WriteOutcome> {
    let root = project_path(&state, &project_id)?;
    let fs = ProjectFs::new(&root)?;
    let backup_root = state.paths.backups_dir().join("edits");
    fs.write_checked(
        &path,
        &content,
        expected_sha256.as_deref(),
        Some(&backup_root),
    )
}

#[tauri::command]
pub fn files_match(
    state: State<'_, AppState>,
    project_id: String,
    query: String,
    limit: Option<usize>,
) -> IhResult<Vec<MentionCandidate>> {
    let root = project_path(&state, &project_id)?;
    let config = state.config_snapshot()?;
    ih_context::match_mentions(
        &root,
        &query,
        limit.unwrap_or(20).clamp(1, 100),
        &config.workspace.exclusions,
    )
}

#[tauri::command]
pub fn context_prepare(
    state: State<'_, AppState>,
    project_id: String,
    paths: Vec<String>,
    allow_sensitive: Option<bool>,
) -> IhResult<PreparedContext> {
    let root = project_path(&state, &project_id)?;
    let options = options(&state, allow_sensitive.unwrap_or(false))?;
    ih_context::prepare(&root, &paths, &options)
}

#[derive(Debug, Serialize)]
pub struct DiffPreview {
    pub id: String,
    pub path: String,
    pub diff: LineDiff,
    pub before_sha256: Option<String>,
    pub additions: usize,
    pub deletions: usize,
}

#[tauri::command]
pub fn diff_prepare(
    state: State<'_, AppState>,
    project_id: String,
    path: String,
    content: String,
) -> IhResult<DiffPreview> {
    let root = project_path(&state, &project_id)?;
    let fs = ProjectFs::new(&root)?;
    let current = fs.metadata(&path)?;
    if current.is_binary {
        return Err(IhError::Invalid(format!("fichier binaire : {path}")));
    }
    let before = fs.read_text(&path, state.config_snapshot()?.workspace.max_editor_bytes)?;
    let diff = diff_lines(&before.content, &content);

    let id = format!("diff-{}", uuid::Uuid::new_v4());
    let preview = DiffPreview {
        id: id.clone(),
        path: path.clone(),
        additions: diff.additions,
        deletions: diff.deletions,
        diff: diff.clone(),
        before_sha256: Some(current.sha256.clone()),
    };
    state
        .pending_diffs
        .lock()
        .map_err(|_| IhError::Internal("registre de diff verrouillé".into()))?
        .insert(
            id,
            crate::state::PendingDiff {
                project_id,
                path,
                before_sha256: current.sha256,
                content,
                created_at: chrono::Utc::now(),
            },
        );
    Ok(preview)
}

#[derive(Debug, Serialize)]
pub struct PendingDiffView {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub created_at: String,
}

#[tauri::command]
pub fn diff_pending(state: State<'_, AppState>) -> IhResult<Vec<PendingDiffView>> {
    let guard = state
        .pending_diffs
        .lock()
        .map_err(|_| IhError::Internal("registre de diff verrouillé".into()))?;
    let mut list: Vec<PendingDiffView> = guard
        .iter()
        .map(|(id, pending)| PendingDiffView {
            id: id.clone(),
            project_id: pending.project_id.clone(),
            path: pending.path.clone(),
            created_at: pending.created_at.to_rfc3339(),
        })
        .collect();
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(list)
}

#[tauri::command]
pub fn diff_reject(state: State<'_, AppState>, id: String) -> IhResult<()> {
    let removed = state
        .pending_diffs
        .lock()
        .map_err(|_| IhError::Internal("registre de diff verrouillé".into()))?
        .remove(&id);
    match removed {
        Some(_) => Ok(()),
        None => Err(IhError::NotFound(format!("modification {id}"))),
    }
}

#[tauri::command]
pub fn diff_accept(state: State<'_, AppState>, id: String) -> IhResult<WriteOutcome> {
    let pending = state
        .pending_diffs
        .lock()
        .map_err(|_| IhError::Internal("registre de diff verrouillé".into()))?
        .remove(&id)
        .ok_or_else(|| IhError::NotFound(format!("modification {id}")))?;

    let root = project_path(&state, &pending.project_id)?;
    let fs = ProjectFs::new(&root)?;
    let backup_root = state.paths.backups_dir().join("diffs");
    // `write_checked` refuse si le fichier a changé depuis la préparation.
    let outcome = fs.write_checked(
        &pending.path,
        &pending.content,
        Some(&pending.before_sha256),
        Some(&backup_root),
    )?;
    Ok(outcome)
}

/// Disposition du workspace (JSON opaque produit par l'interface).
#[tauri::command]
pub fn layout_get(state: State<'_, AppState>) -> IhResult<Option<String>> {
    state.store.get_setting(LAYOUT_KEY)
}

#[tauri::command]
pub fn layout_save(state: State<'_, AppState>, layout: String) -> IhResult<()> {
    if layout.len() > MAX_LAYOUT_BYTES {
        return Err(IhError::Invalid(
            "disposition trop volumineuse (64 Kio max)".into(),
        ));
    }
    // Vérifie que c'est bien du JSON avant de persister.
    let value: serde_json::Value = serde_json::from_str(&layout)
        .map_err(|e| IhError::Invalid(format!("disposition JSON invalide : {e}")))?;
    state.store.set_setting(LAYOUT_KEY, &value.to_string())
}
