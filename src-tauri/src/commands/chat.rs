//! Commandes de conversation : envoi avec streaming, annulation, historique.

use crate::state::AppState;
use ih_chat::{cancel_channel, run_turn, ChatTurn, DEFAULT_THREAD_TITLE};
use ih_core::{IhError, IhResult, ProjectId, ThreadId};
use ih_providers::build_provider;
use ih_security::secrets::provider_account;
use serde::Serialize;
use tauri::{Emitter, State};

/// Accusé de réception immédiat : le flux arrive ensuite par événements.
#[derive(Debug, Serialize)]
pub struct ChatAccepted {
    pub request_id: String,
    pub thread_id: String,
    pub project_id: String,
    pub provider_id: String,
    pub model: String,
}

/// Démarre un tour de conversation et renvoie immédiatement de quoi annuler.
///
/// Événements émis :
/// - `ih://chat-delta` `{ request_id, thread_id, delta }`
/// - `ih://chat-done`  `{ request_id, thread_id, outcome }`
/// - `ih://chat-error` `{ request_id, thread_id, message, code }`
#[tauri::command]
// Les paramètres correspondent un à un aux champs du formulaire de l'interface :
// les regrouper dans une struct n'apporterait rien et alourdirait l'appel IPC.
#[allow(clippy::too_many_arguments)]
pub async fn chat_send(
    state: State<'_, AppState>,
    project_id: String,
    thread_id: Option<String>,
    provider_id: String,
    model: String,
    prompt: String,
    system: Option<String>,
    history_limit: Option<usize>,
) -> IhResult<ChatAccepted> {
    if prompt.trim().is_empty() {
        return Err(IhError::Invalid("message vide".into()));
    }
    let project = ProjectId::parse(project_id)?;
    if state.store.get_project(&project)?.is_none() {
        return Err(IhError::NotFound(format!("projet {project}")));
    }

    let config = state.config_snapshot()?;
    let provider_config = config
        .provider(&provider_id)
        .ok_or_else(|| IhError::NotFound(format!("fournisseur {provider_id}")))?
        .clone();
    if !provider_config.enabled {
        return Err(IhError::Denied(format!(
            "le fournisseur {provider_id} est désactivé"
        )));
    }
    let secret = state.secrets.get(&provider_account(&provider_id))?;
    // Échoue avant tout travail si la clé manque (fournisseur cloud).
    let provider = build_provider(&provider_config, secret)?;

    let thread = match thread_id {
        Some(id) => {
            let id = ThreadId::parse(id)?;
            let existing = state
                .store
                .get_thread(&id)?
                .ok_or_else(|| IhError::NotFound(format!("conversation {id}")))?;
            if existing.project_id != project {
                return Err(IhError::Denied(
                    "la conversation n'appartient pas à ce projet".into(),
                ));
            }
            existing
        }
        None => state
            .store
            .create_thread(&project, DEFAULT_THREAD_TITLE, Some(&provider_id))?,
    };

    let mut turn = ChatTurn::new(
        thread.id.clone(),
        provider_id.clone(),
        model.clone(),
        prompt,
    );
    turn.system = system;
    if let Some(limit) = history_limit {
        turn.history_limit = limit;
    }

    let (canceller, token) = cancel_channel();
    let request_id = format!("req-{}", uuid::Uuid::new_v4());
    let thread_for_task = thread.id.clone();
    let request_for_task = request_id.clone();
    let store = state.store.clone();
    let handle = state.app_handle.clone();
    let registry = state.chat.clone();

    state
        .chat
        .lock()
        .map_err(|_| IhError::Internal("registre de chat verrouillé".into()))?
        .insert(request_id.clone(), canceller);

    tauri::async_runtime::spawn(async move {
        let mut sink = |delta: &str| {
            let _ = handle.emit(
                "ih://chat-delta",
                serde_json::json!({
                    "request_id": request_for_task,
                    "thread_id": thread_for_task,
                    "delta": delta,
                }),
            );
        };
        let result = run_turn(provider.as_ref(), store.as_ref(), &turn, token, &mut sink).await;
        match result {
            Ok(outcome) => {
                let _ = handle.emit(
                    "ih://chat-done",
                    serde_json::json!({
                        "request_id": request_for_task,
                        "thread_id": thread_for_task,
                        "outcome": outcome,
                    }),
                );
            }
            Err(error) => {
                let _ = handle.emit(
                    "ih://chat-error",
                    serde_json::json!({
                        "request_id": request_for_task,
                        "thread_id": thread_for_task,
                        "code": error.code(),
                        "message": error.to_string(),
                    }),
                );
            }
        }
        // Le tour est terminé : l'entrée ne doit pas rester dans le registre,
        // sinon `chat_active` compterait des générations mortes.
        if let Ok(mut guard) = registry.lock() {
            guard.remove(&request_for_task);
        }
    });

    Ok(ChatAccepted {
        request_id,
        thread_id: thread.id.into_string(),
        project_id: project.into_string(),
        provider_id,
        model,
    })
}

/// Annule un tour en cours. Renvoie `false` si la génération est déjà finie.
#[tauri::command]
pub fn chat_cancel(state: State<'_, AppState>, request_id: String) -> IhResult<bool> {
    let canceller = state
        .chat
        .lock()
        .map_err(|_| IhError::Internal("registre de chat verrouillé".into()))?
        .remove(&request_id);
    Ok(match canceller {
        Some(canceller) => canceller.cancel(),
        None => false,
    })
}

/// Nombre de générations actives (indicateur de ressources de l'interface).
#[tauri::command]
pub fn chat_active(state: State<'_, AppState>) -> IhResult<usize> {
    Ok(state
        .chat
        .lock()
        .map_err(|_| IhError::Internal("registre de chat verrouillé".into()))?
        .len())
}

/// Renomme une conversation.
#[tauri::command]
pub fn thread_rename(state: State<'_, AppState>, id: String, title: String) -> IhResult<()> {
    let id = ThreadId::parse(id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err(IhError::Invalid("titre vide".into()));
    }
    if title.chars().count() > 200 {
        return Err(IhError::Invalid(
            "titre trop long (200 caractères max)".into(),
        ));
    }
    state.store.rename_thread(&id, title)
}

/// Recherche des conversations par titre ou contenu.
#[tauri::command]
pub fn threads_search(
    state: State<'_, AppState>,
    project_id: String,
    query: String,
    limit: Option<usize>,
) -> IhResult<Vec<ih_store::Thread>> {
    let project = ProjectId::parse(project_id)?;
    ih_chat::search_threads(
        &state.store,
        &project,
        &query,
        limit.unwrap_or(50).clamp(1, 200),
    )
}
