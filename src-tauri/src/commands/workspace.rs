//! Commandes Git et terminal.

use super::project_path;
use crate::state::AppState;
use ih_core::{IhError, IhResult};
use ih_workspace::git::{CommitInfo, GitRepo, GitStatus, Worktree};
use ih_workspace::pty::PtySession;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, State};

pub struct PtyEntry {
    pub session: PtySession,
    pub stop: Arc<AtomicBool>,
}

fn repo(state: &AppState, project_id: &str) -> IhResult<GitRepo> {
    let root = project_path(state, project_id)?;
    GitRepo::discover(&root)
}

#[tauri::command]
pub fn git_status(state: State<'_, AppState>, project_id: String) -> IhResult<GitStatus> {
    repo(&state, &project_id)?.status()
}

#[tauri::command]
pub fn git_diff(
    state: State<'_, AppState>,
    project_id: String,
    staged: bool,
    path: Option<String>,
) -> IhResult<String> {
    repo(&state, &project_id)?.diff(staged, path.as_deref())
}

#[tauri::command]
pub fn git_stage(
    state: State<'_, AppState>,
    project_id: String,
    paths: Vec<String>,
) -> IhResult<()> {
    repo(&state, &project_id)?.stage(&paths)
}

#[tauri::command]
pub fn git_unstage(
    state: State<'_, AppState>,
    project_id: String,
    paths: Vec<String>,
) -> IhResult<()> {
    repo(&state, &project_id)?.unstage(&paths)
}

#[tauri::command]
pub fn git_commit(
    state: State<'_, AppState>,
    project_id: String,
    message: String,
) -> IhResult<String> {
    repo(&state, &project_id)?.commit(&message)
}

#[tauri::command]
pub fn git_branches(state: State<'_, AppState>, project_id: String) -> IhResult<Vec<String>> {
    repo(&state, &project_id)?.branches()
}

#[tauri::command]
pub fn git_log(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<u32>,
) -> IhResult<Vec<CommitInfo>> {
    repo(&state, &project_id)?.log(limit.unwrap_or(30).clamp(1, 500))
}

#[tauri::command]
pub fn git_worktrees(state: State<'_, AppState>, project_id: String) -> IhResult<Vec<Worktree>> {
    repo(&state, &project_id)?.worktrees()
}

#[tauri::command]
pub fn git_add_worktree(
    state: State<'_, AppState>,
    project_id: String,
    target: String,
    branch: String,
    create: bool,
) -> IhResult<()> {
    let target_path = std::path::PathBuf::from(target);
    if !target_path.is_absolute() {
        return Err(IhError::Invalid(
            "le chemin du worktree doit être absolu".into(),
        ));
    }
    repo(&state, &project_id)?.add_worktree(&target_path, &branch, create)
}

#[tauri::command]
pub fn git_remove_worktree(
    state: State<'_, AppState>,
    project_id: String,
    target: String,
    force: bool,
) -> IhResult<()> {
    repo(&state, &project_id)?.remove_worktree(std::path::Path::new(&target), force)
}

// ----------------------------------------------------------------- terminal

#[tauri::command]
pub fn terminal_open(
    state: State<'_, AppState>,
    project_id: String,
    cols: Option<u16>,
    rows: Option<u16>,
) -> IhResult<String> {
    let root = project_path(&state, &project_id)?;
    let config = state.config_snapshot()?;
    let shell = config.terminal.shell.clone();
    let session = PtySession::open(
        &shell,
        &config.terminal.args,
        &root,
        cols.unwrap_or(120).clamp(20, 400),
        rows.unwrap_or(32).clamp(5, 200),
        &[("TERM".into(), "xterm-256color".into())],
    )?;
    let id = session.id().to_string();
    let reader = session.try_clone_reader()?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = stop.clone();
    let app = state.app_handle.clone();
    let session_id = id.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buffer = [0u8; 8192];
        while !stop_reader.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buffer[..n]).to_string();
                    let _ = app.emit(
                        "ih://pty-output",
                        serde_json::json!({ "id": session_id, "data": data }),
                    );
                }
                Err(_) => break,
            }
        }
        let _ = app.emit("ih://pty-exit", serde_json::json!({ "id": session_id }));
    });

    state
        .pty
        .lock()
        .map_err(|_| IhError::Internal("registre de terminaux verrouillé".into()))?
        .insert(id.clone(), PtyEntry { session, stop });
    Ok(id)
}

#[tauri::command]
pub fn terminal_write(state: State<'_, AppState>, id: String, data: String) -> IhResult<()> {
    let mut guard = state
        .pty
        .lock()
        .map_err(|_| IhError::Internal("registre de terminaux verrouillé".into()))?;
    let entry = guard
        .get_mut(&id)
        .ok_or_else(|| IhError::NotFound(format!("terminal {id}")))?;
    entry.session.write(&data)
}

#[tauri::command]
pub fn terminal_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> IhResult<()> {
    let mut guard = state
        .pty
        .lock()
        .map_err(|_| IhError::Internal("registre de terminaux verrouillé".into()))?;
    let entry = guard
        .get_mut(&id)
        .ok_or_else(|| IhError::NotFound(format!("terminal {id}")))?;
    entry.session.resize(cols, rows)
}

#[tauri::command]
pub fn terminal_close(state: State<'_, AppState>, id: String) -> IhResult<()> {
    let mut guard = state
        .pty
        .lock()
        .map_err(|_| IhError::Internal("registre de terminaux verrouillé".into()))?;
    if let Some(mut entry) = guard.remove(&id) {
        entry.stop.store(true, Ordering::SeqCst);
        entry.session.kill()?;
    }
    Ok(())
}

#[allow(unused_imports)]
use std::io::Read;
