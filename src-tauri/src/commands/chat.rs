//! Commandes de conversation : envoi avec streaming, annulation, historique.

use crate::state::AppState;
use ih_chat::{cancel_channel, mcp_proposal, run_turn, ChatTurn, DEFAULT_THREAD_TITLE};
use ih_core::{IhError, IhResult, ProjectId, ThreadId};
use ih_providers::build_provider;
use ih_security::secrets::provider_account;
use ih_store::{Message, NewMessage};
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
    // Chemins relatifs des fichiers joints comme contexte : le contenu est lu
    // et assemblé par le noyau (`ih-context`), jamais par l'interface.
    context_paths: Option<Vec<String>>,
    allow_sensitive_context: Option<bool>,
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

    // Outils MCP activés : proposés au modèle dans le prompt système, jamais
    // exécutés par lui (l'approbation est exigée plus tard).
    let mcp_tools = mcp_tools_addendum(&state);
    let mut turn = ChatTurn::new(
        thread.id.clone(),
        provider_id.clone(),
        model.clone(),
        prompt,
    );
    turn.system = match (system, mcp_tools.as_ref().map(|(text, _)| text.clone())) {
        (Some(system), Some(addendum)) => Some(format!("{system}\n\n{addendum}")),
        (Some(system), None) => Some(system),
        (None, Some(addendum)) => Some(addendum),
        (None, None) => None,
    };
    let enabled_pairs = mcp_tools.map(|(_, pairs)| pairs).unwrap_or_default();
    if let Some(paths) = context_paths {
        if !paths.is_empty() {
            let config = state.config_snapshot()?;
            let options = ih_context::ContextOptions {
                max_files: config.workspace.max_context_files,
                max_bytes_per_file: config.workspace.max_context_bytes_per_file,
                max_total_bytes: config.workspace.max_context_bytes,
                allow_sensitive: allow_sensitive_context.unwrap_or(false),
                extra_exclusions: config.workspace.exclusions.clone(),
            };
            let root = super::project_path(&state, project.as_str())?;
            let prepared = ih_context::prepare(&root, &paths, &options)?;
            if !prepared.prompt_block.trim().is_empty() {
                turn.context = Some(prepared.prompt_block);
            }
        }
    }
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
                if !outcome.cancelled && !enabled_pairs.is_empty() {
                    let proposals = mcp_proposal::extract_proposals(&outcome.text, &enabled_pairs);
                    if !proposals.is_empty() {
                        let _ = handle.emit(
                            "ih://chat-mcp-proposal",
                            serde_json::json!({
                                "request_id": request_for_task,
                                "thread_id": thread_for_task,
                                "proposals": proposals,
                            }),
                        );
                    }
                }
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
/// Construit la description des outils MCP activés proposée au modèle, et la
/// liste des couples (serveur, outil) autorisés pour filtrer les propositions.
fn mcp_tools_addendum(state: &AppState) -> Option<(String, Vec<(String, String)>)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut budget = 8 * 1024usize;
    let servers = state.store.list_mcp_servers().ok()?;
    for server in servers.iter().filter(|server| server.enabled) {
        let tools = match state.store.list_mcp_tools(Some(&server.id)) {
            Ok(tools) => tools,
            Err(_) => continue,
        };
        for tool in tools.into_iter().filter(|tool| tool.enabled) {
            if pairs.len() >= 16 || budget < 200 {
                break;
            }
            let schema = serde_json::to_string(&tool.input_schema).unwrap_or_default();
            let line = format!(
                "- {}/{} (risque {}): {} | schéma: {}",
                server.id,
                tool.name,
                tool.risk,
                truncate_text(&tool.description, 160),
                truncate_text(&schema, 500)
            );
            budget = budget.saturating_sub(line.len() + 1);
            pairs.push((server.id.clone(), tool.name.clone()));
            lines.push(line);
        }
    }
    if pairs.is_empty() {
        return None;
    }
    let text = format!(
        "Outils MCP réellement activés par l'utilisateur, proposables (tu ne peux pas les exécuter toi-même) :\n{}\n\nPour proposer un appel, termine ta réponse par un bloc de code JSON étiqueté ih-mcp-call : {{\"server_id\":\"…\",\"tool\":\"…\",\"arguments\":{{…}}}}. L'utilisateur devra approuver l'appel ; n'invente jamais d'outil hors de la liste.",
        lines.join("\n")
    );
    Some((text, pairs))
}

fn truncate_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "…"
}

/// Ajoute au fil un message « outil » (résultat d'un appel MCP approuvé).
///
/// Le contenu est borné ; le statut est validé. Ce message n'est pas renvoyé au
/// modèle dans l'historique — le tour suivant repart de la conversation.
#[tauri::command]
pub fn chat_append_tool_result(
    state: State<'_, AppState>,
    thread_id: String,
    tool: String,
    status: String,
    content: String,
) -> IhResult<Message> {
    let thread = ThreadId::parse(thread_id)?;
    state
        .store
        .get_thread(&thread)?
        .ok_or_else(|| IhError::NotFound(format!("conversation {thread}")))?;
    if tool.trim().is_empty() {
        return Err(IhError::Invalid("outil MCP vide".into()));
    }
    if !matches!(status.as_str(), "completed" | "failed" | "denied") {
        return Err(IhError::Invalid(format!(
            "statut d'appel inconnu : {status}"
        )));
    }
    let bounded = truncate_text(&content, 32 * 1024);
    state.store.append_message(&NewMessage {
        thread_id: thread,
        role: "tool".into(),
        content: bounded,
        metadata: Some(serde_json::json!({
            "source": "mcp",
            "tool": tool.trim(),
            "status": status,
        })),
    })
}

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
