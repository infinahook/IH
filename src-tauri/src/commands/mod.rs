//! Commandes IPC exposées à l'interface React.

pub mod agent;
pub mod ai;
pub mod chat;
pub mod core;
pub mod extensions;
pub mod files;
pub mod mcp;
pub mod mcp_tools;
pub mod ops;
pub mod team;
pub mod workspace;

use crate::state::AppState;
use ih_core::{IhError, IhResult, ProjectId};
use std::path::PathBuf;

/// Résout le dossier d'un projet enregistré, en vérifiant qu'il existe encore.
pub fn project_path(state: &AppState, id: &str) -> IhResult<PathBuf> {
    let project_id = ProjectId::parse(id)?;
    let project = state
        .store
        .get_project(&project_id)?
        .ok_or_else(|| IhError::NotFound(format!("projet {id}")))?;
    let path = PathBuf::from(&project.path);
    if !path.is_dir() {
        return Err(IhError::NotFound(format!(
            "le dossier du projet n'existe plus : {}",
            project.path
        )));
    }
    Ok(path)
}
