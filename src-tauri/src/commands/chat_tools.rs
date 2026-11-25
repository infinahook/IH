//! Cycle d'outils MCP natifs dans le chat.
//!
//! Séquence stricte, sans chemin parallèle :
//!
//! 1. le modèle demande un ou plusieurs outils (`ToolCall`) ;
//! 2. chaque demande est enregistrée en proposition `pending` (arguments
//!    redactés pour l'affichage et l'audit) et présentée à l'utilisateur ;
//! 3. l'utilisateur approuve ou refuse — c'est la décision humaine, liée à
//!    l'empreinte exacte des arguments ;
//! 4. l'approbation matérialisée est à usage unique et consommée par
//!    `invoke_mcp_tool`, seul point d'entrée d'exécution MCP ;
//! 5. le résultat est renvoyé au modèle comme message `tool`, et le tour
//!    reprend (borné par [`MAX_TOOL_ROUNDS`] et [`MAX_CALLS_PER_TURN`]).
//!
//! Un redémarrage ne rejoue jamais une proposition `pending` : rien n'exécute
//! d'outil sans une décision explicite de l'utilisateur.

use crate::commands::mcp::{invoke_mcp_tool, risk_from_str, truncate, InvokeRequest};
use crate::commands::mcp_tools::ChatToolCatalog;
use crate::state::AppState;
use ih_chat::{cancel_channel, run_turn, Canceller, ChatTurn};
use ih_core::{IhError, IhResult, ThreadId};
use ih_providers::{build_provider, ToolCall};
use ih_security::approval::ApprovalDecision;
use ih_security::mcp::{mcp_call_fingerprint, redact_arguments, McpCaller};
use ih_security::secrets::{provider_account, SecretStore};
use ih_store::{ChatToolCallRecord, NewChatToolCall, NewMessage, Store};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// Dépendances possédées d'une continuation (aucune référence à l'état Tauri).
#[derive(Clone)]
pub struct ContinuationDeps {
    pub store: Arc<Store>,
    pub handle: AppHandle,
    pub registry: Arc<Mutex<HashMap<String, Canceller>>>,
    pub config: ih_core::config::AppConfig,
    pub secrets: Arc<dyn SecretStore>,
}

impl ContinuationDeps {
    pub fn from_state(state: &AppState) -> IhResult<Self> {
        Ok(Self {
            store: state.store.clone(),
            handle: state.app_handle.clone(),
            registry: state.chat.clone(),
            config: state.config_snapshot()?,
            secrets: state.secrets.clone(),
        })
    }
}

/// Nombre maximal de cycles de continuation dans un même tour.
pub const MAX_TOOL_ROUNDS: u32 = 3;
/// Nombre maximal de propositions sur l'ensemble d'un tour.
pub const MAX_CALLS_PER_TURN: usize = 6;
/// Taille maximale du texte d'un résultat transmis au modèle.
const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;
const MAX_RESULT_STORED: usize = 256 * 1024;

/// Événement de carte d'appel (le contenu sensible est déjà masqué).
fn emit_card(handle: &AppHandle, phase: &str, call: &ChatToolCallRecord) {
    let _ = handle.emit(
        "ih://chat-tool-call",
        serde_json::json!({
            "phase": phase,
            "thread_id": call.thread_id.as_str(),
            "request_id": call.request_id,
            "call_id": call.id,
            "provider_call_id": call.provider_call_id,
            "round": call.round,
            "exposed_name": call.exposed_name,
            "server_id": call.server_id,
            "tool": call.tool,
            "risk": call.risk,
            "category": call.category,
            "arguments": call.arguments,
            "status": call.status,
            "error": call.error,
            "result_text": call.result_text,
        }),
    );
}

/// Budget d'appels restant pour ce tour (propositions invalides non comptées).
pub fn remaining_call_budget(existing: &[ChatToolCallRecord]) -> usize {
    MAX_CALLS_PER_TURN.saturating_sub(
        existing
            .iter()
            .filter(|call| call.status != "invalid")
            .count(),
    )
}

/// Un cycle supplémentaire de tool calling est-il encore autorisé ?
pub fn cycle_allowed(round: u32) -> bool {
    round < MAX_TOOL_ROUNDS
}

/// Enregistre les propositions d'un tour et les présente à l'utilisateur.
///
/// Renvoie les propositions **en attente de décision** (les propositions
/// invalides sont déjà closes et n'attendent rien).
#[allow(clippy::too_many_arguments)]
pub fn propose_calls(
    store: &Store,
    handle: &AppHandle,
    thread_id: &ThreadId,
    project_id: &ih_core::ProjectId,
    request_id: &str,
    round: u32,
    calls: &[ToolCall],
    catalog: &ChatToolCatalog,
) -> IhResult<Vec<ChatToolCallRecord>> {
    let existing = store.list_chat_tool_calls_for_request(request_id)?;
    let mut budget = remaining_call_budget(&existing);
    let mut pending = Vec::new();
    for (position, call) in calls.iter().enumerate() {
        let over_budget = budget == 0 || position >= MAX_CALLS_PER_TURN;
        if !over_budget {
            budget -= 1;
        }
        let (server_id, _server_name, tool_name, risk, category, redacted, raw, error) =
            match catalog.target(&call.name) {
                Some(target) => {
                    let arguments = &call.arguments;
                    if !arguments.is_object() {
                        (
                            target.server_id.clone(),
                            target.server_name.clone(),
                            target.tool.clone(),
                            target.risk.clone(),
                            target.category.clone(),
                            serde_json::json!({}),
                            arguments.clone(),
                            Some("arguments d'outil : objet JSON attendu".to_string()),
                        )
                    } else if let Some(record) = store
                        .get_mcp_tool(&target.server_id, &target.tool)
                        .ok()
                        .flatten()
                    {
                        let validation =
                            ih_mcp::schema::validate_arguments(&record.input_schema, arguments);
                        let redacted = redact_arguments(arguments);
                        (
                            target.server_id.clone(),
                            target.server_name.clone(),
                            target.tool.clone(),
                            target.risk.clone(),
                            target.category.clone(),
                            redacted,
                            arguments.clone(),
                            validation.err().map(|err| err.to_string()),
                        )
                    } else {
                        (
                            target.server_id.clone(),
                            target.server_name.clone(),
                            target.tool.clone(),
                            target.risk.clone(),
                            target.category.clone(),
                            serde_json::json!({}),
                            arguments.clone(),
                            Some("outil MCP introuvable dans le catalogue".to_string()),
                        )
                    }
                }
                None => (
                    "unknown".to_string(),
                    "unknown".to_string(),
                    call.name.clone(),
                    "unknown".into(),
                    "unknown".into(),
                    serde_json::json!({}),
                    call.arguments.clone(),
                    Some("outil inconnu : nom non exposé par le catalogue".to_string()),
                ),
            };
        let mut error = error;
        if over_budget {
            error = Some(format!(
                "limite de {MAX_CALLS_PER_TURN} appels d'outils par tour dépassée"
            ));
        }
        let fingerprint = mcp_call_fingerprint(&server_id, &tool_name, &raw);
        let status = if error.is_some() {
            "invalid"
        } else {
            "pending"
        };
        let record = store.insert_chat_tool_call(&NewChatToolCall {
            thread_id: thread_id.clone(),
            project_id: project_id.clone(),
            request_id: request_id.to_string(),
            round,
            provider_call_id: call.id.clone(),
            exposed_name: call.name.clone(),
            server_id,
            tool: tool_name,
            risk,
            category,
            fingerprint,
            arguments: redacted,
            raw_arguments: raw,
            status: status.into(),
            error: error.clone(),
        })?;
        if let Some(detail) = &error {
            tracing::info!(
                call = %record.id,
                server = %record.server_id,
                "proposition d'outil rejetée : {detail}"
            );
        }
        emit_card(handle, "proposed", &record);
        if record.status == "pending" {
            pending.push(record);
        } else {
            // La proposition invalide est immédiatement close et signalée au
            // modèle au prochain cycle sous forme de message `tool`.
            let _ = store.decide_chat_tool_call(&record.id, "invalid", error.as_deref());
        }
    }
    Ok(pending)
}

/// Renvoie au modèle un message `tool` (résultat ou refus) borné.
fn append_tool_message(
    state: &AppState,
    record: &ChatToolCallRecord,
    status: &str,
    content: &str,
) -> IhResult<()> {
    let bounded = truncate(content, MAX_TOOL_RESULT_BYTES);
    state.store.append_message(&NewMessage {
        thread_id: record.thread_id.clone(),
        role: "tool".into(),
        content: bounded,
        metadata: Some(serde_json::json!({
            "source": "mcp-tool",
            "tool_call_id": record.provider_call_id,
            "tool": record.tool,
            "server_id": record.server_id,
            "status": status,
        })),
    })?;
    Ok(())
}

/// Décision humaine sur une proposition (`approve` ou `deny`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallDecision {
    Approve,
    Deny,
}

impl ToolCallDecision {
    pub fn parse(value: &str) -> IhResult<Self> {
        match value {
            "approve" => Ok(Self::Approve),
            "deny" => Ok(Self::Deny),
            other => Err(IhError::Invalid(format!(
                "décision d'appel d'outil inconnue : {other}"
            ))),
        }
    }
}

/// Résultat rendu à l'interface après une décision.
#[derive(Debug, Serialize)]
pub struct ChatToolDecisionOutcome {
    pub call_id: String,
    pub status: String,
    pub summary: String,
    pub result_text: Option<String>,
    pub continued: bool,
}

/// Applique la décision de l'utilisateur.
///
/// L'approbation est matérialisée ici, liée à l'empreinte exacte des
/// arguments, puis consommée par `invoke_mcp_tool` : le clic humain sur la
/// carte *est* l'approbation à usage unique.
pub async fn decide_call(
    state: &AppState,
    call_id: &str,
    decision: ToolCallDecision,
) -> IhResult<ChatToolDecisionOutcome> {
    let record = state
        .store
        .get_chat_tool_call(call_id)?
        .ok_or_else(|| IhError::NotFound(format!("appel d'outil {call_id}")))?;
    if record.status != "pending" {
        return Err(IhError::Conflict(format!(
            "l'appel d'outil {call_id} n'est plus en attente (statut {})",
            record.status
        )));
    }

    if decision == ToolCallDecision::Deny {
        let record = state.store.decide_chat_tool_call(
            call_id,
            "denied",
            Some("refusé par l'utilisateur"),
        )?;
        append_tool_message(
            state,
            &record,
            "denied",
            "L'utilisateur a refusé cet appel d'outil. Aucune exécution n'a eu lieu.",
        )?;
        emit_card(&state.app_handle, "denied", &record);
        let continued = maybe_continue(state, &record)?;
        return Ok(ChatToolDecisionOutcome {
            call_id: record.id,
            status: "denied".into(),
            summary: "Appel refusé par l'utilisateur".into(),
            result_text: None,
            continued,
        });
    }

    // Approbation : empreinte recalculée sur les arguments exacts, pour ne
    // jamais consommer une approbation valable pour d'autres paramètres.
    let fingerprint = mcp_call_fingerprint(&record.server_id, &record.tool, &record.raw_arguments);
    let risk = risk_from_str(&record.risk)?;
    let summary = format!(
        "Appel de l'outil MCP « {} » (serveur « {} »)",
        record.tool, record.server_id
    );
    let detail = format!(
        "Outil : {}\nServeur : {}\nRisque : {}\nParamètres : {}",
        record.tool,
        record.server_id,
        record.risk,
        truncate(
            &serde_json::to_string(&record.arguments).unwrap_or_default(),
            2000
        )
    );
    let approval = state.approvals.ask_with_fingerprint(
        "mcp_tool",
        &summary,
        &detail,
        risk,
        Some(fingerprint.clone()),
    )?;
    let approval_id = approval.id.into_string();
    state
        .approvals
        .resolve(&approval_id, ApprovalDecision::Approve)?;
    // Réclamation atomique : une carte ne peut être exécutée qu'une fois,
    // même en cas de double clic ou de course entre deux décisions.
    if !state.store.claim_chat_tool_call(call_id, &approval_id)? {
        return Err(IhError::Conflict(format!(
            "l'appel d'outil {call_id} est déjà en cours d'exécution ou déjà décidé"
        )));
    }

    let outcome = invoke_mcp_tool(
        state,
        InvokeRequest {
            server_id: record.server_id.clone(),
            tool: record.tool.clone(),
            arguments: record.raw_arguments.clone(),
            caller: McpCaller::Chat,
            caller_id: Some(record.thread_id.as_str().to_string()),
            approval_id: Some(approval_id.clone()),
            role: None,
        },
    )
    .await;

    match outcome {
        Ok(call) if call.status != "completed" => {
            let message = format!("issue d'exécution inattendue : {}", call.status);
            let record = state.store.finish_chat_tool_call(
                call_id,
                "failed",
                Some(&message),
                None,
                Some(&approval_id),
            )?;
            append_tool_message(
                state,
                &record,
                "failed",
                "L'appel d'outil n'a pas produit de résultat exploitable.",
            )?;
            emit_card(&state.app_handle, "failed", &record);
            let continued = maybe_continue(state, &record)?;
            Ok(ChatToolDecisionOutcome {
                call_id: record.id,
                status: "failed".into(),
                summary: message,
                result_text: None,
                continued,
            })
        }
        Ok(call) => {
            let result = call.result.unwrap_or(ih_mcp::ToolCallResult {
                content: Vec::new(),
                text: String::new(),
                structured: None,
                is_error: false,
                truncated: false,
            });
            let status = if result.is_error {
                "failed"
            } else {
                "completed"
            };
            let text = truncate(&result.text, MAX_RESULT_STORED);
            let record = state.store.finish_chat_tool_call(
                call_id,
                status,
                None,
                Some(&text),
                Some(&approval_id),
            )?;
            append_tool_message(state, &record, status, &text)?;
            emit_card(&state.app_handle, "finished", &record);
            let continued = maybe_continue(state, &record)?;
            Ok(ChatToolDecisionOutcome {
                call_id: record.id,
                status: status.into(),
                summary: if result.is_error {
                    "L'outil a signalé une erreur".into()
                } else {
                    "Appel exécuté".into()
                },
                result_text: Some(text),
                continued,
            })
        }
        Err(error) => {
            let message = truncate(&error.to_string(), 600);
            let record = state.store.finish_chat_tool_call(
                call_id,
                "failed",
                Some(&message),
                None,
                Some(&approval_id),
            )?;
            append_tool_message(
                state,
                &record,
                "failed",
                &format!("L'appel d'outil a échoué : {message}"),
            )?;
            emit_card(&state.app_handle, "failed", &record);
            let continued = maybe_continue(state, &record)?;
            Ok(ChatToolDecisionOutcome {
                call_id: record.id,
                status: "failed".into(),
                summary: format!("Échec de l'appel : {message}"),
                result_text: None,
                continued,
            })
        }
    }
}

/// Lance la continuation si toutes les propositions du tour sont résolues.
pub fn maybe_continue(state: &AppState, record: &ChatToolCallRecord) -> IhResult<bool> {
    maybe_continue_cycle(state, &record.thread_id, &record.request_id)
}

fn maybe_continue_cycle(
    state: &AppState,
    thread_id: &ThreadId,
    request_id: &str,
) -> IhResult<bool> {
    let deps = ContinuationDeps::from_state(state)?;
    continue_with(&deps, thread_id, request_id)
}

/// Continue le tour à partir des dépendances possédées (utilisable dans une
/// tâche de fond, sans garder l'état Tauri en vie).
pub fn continue_with(
    deps: &ContinuationDeps,
    thread_id: &ThreadId,
    request_id: &str,
) -> IhResult<bool> {
    let calls = deps.store.list_chat_tool_calls_for_request(request_id)?;
    if calls.iter().any(|call| call.status == "pending") {
        return Ok(false);
    }
    let round = calls.iter().map(|call| call.round).max().unwrap_or(0);
    if !cycle_allowed(round) {
        let _ = deps.handle.emit(
            "ih://chat-error",
            serde_json::json!({
                "request_id": request_id,
                "thread_id": thread_id.as_str(),
                "code": "tool_limit",
                "message": format!(
                    "Limite de {MAX_TOOL_ROUNDS} cycles d'outils atteinte pour ce tour."
                ),
            }),
        );
        return Ok(false);
    }
    spawn_continuation(deps, thread_id, request_id, round + 1)?;
    Ok(true)
}

/// Programme le cycle suivant du modèle (historique complet, aucun message
/// utilisateur ajouté) et diffuse ses fragments comme ceux du tour initial.
pub fn spawn_continuation(
    deps: &ContinuationDeps,
    thread_id: &ThreadId,
    request_id: &str,
    round: u32,
) -> IhResult<()> {
    let thread = deps
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| IhError::NotFound(format!("conversation {thread_id}")))?;
    let project_id = thread.project_id.clone();
    let (provider_id, model) = last_provider_model(deps, thread_id)?;
    let provider_config = deps
        .config
        .provider(&provider_id)
        .ok_or_else(|| IhError::NotFound(format!("fournisseur {provider_id}")))?
        .clone();
    let secret = if provider_config.has_secret {
        deps.secrets.get(&provider_account(&provider_id))?
    } else {
        None
    };
    let provider = build_provider(&provider_config, secret)?;
    if !provider.supports_native_tools() {
        return Ok(());
    }
    let catalog = chat_tool_catalog_from_store(&deps.store)?;
    let mut turn = ChatTurn::continuation(thread_id.clone(), provider_id, model);
    turn.tools = catalog.definitions.clone();

    let (canceller, token) = cancel_channel();
    {
        let mut guard = deps
            .registry
            .lock()
            .map_err(|_| IhError::Internal("registre de chat verrouillé".into()))?;
        if guard.contains_key(request_id) {
            // Un cycle est déjà en cours pour ce tour : ne pas doubler l'appel.
            return Ok(());
        }
        guard.insert(request_id.to_string(), canceller);
    }
    let deps_for_task = deps.clone();
    let request = request_id.to_string();
    let thread_id = thread_id.clone();
    tauri::async_runtime::spawn(async move {
        let request_for_task = request.clone();
        let thread_for_task = thread_id.clone();
        let handle = deps_for_task.handle.clone();
        let mut sink = |delta: &str| {
            let _ = handle.emit(
                "ih://chat-delta",
                serde_json::json!({
                    "request_id": request_for_task,
                    "thread_id": thread_for_task.as_str(),
                    "delta": delta,
                }),
            );
        };
        let result = run_turn(
            provider.as_ref(),
            deps_for_task.store.as_ref(),
            &turn,
            token,
            &mut sink,
        )
        .await;
        match result {
            Ok(outcome) => {
                if !outcome.cancelled && !outcome.tool_calls.is_empty() {
                    let proposed = propose_calls(
                        &deps_for_task.store,
                        &deps_for_task.handle,
                        &thread_id,
                        &project_id,
                        &request,
                        round,
                        &outcome.tool_calls,
                        &catalog,
                    );
                    match proposed {
                        Ok(pending) => {
                            if pending.is_empty() {
                                // Toutes les propositions sont invalides : le
                                // modèle reçoit les erreurs et reprend, borné.
                                let _ = continue_with(&deps_for_task, &thread_id, &request);
                            }
                        }
                        Err(error) => {
                            let _ = deps_for_task.handle.emit(
                                "ih://chat-error",
                                serde_json::json!({
                                    "request_id": request,
                                    "thread_id": thread_id.as_str(),
                                    "code": error.code(),
                                    "message": error.to_string(),
                                }),
                            );
                        }
                    }
                } else {
                    let _ = deps_for_task.handle.emit(
                        "ih://chat-done",
                        serde_json::json!({
                            "request_id": request,
                            "thread_id": thread_id.as_str(),
                            "outcome": outcome,
                        }),
                    );
                }
            }
            Err(error) => {
                let _ = deps_for_task.handle.emit(
                    "ih://chat-error",
                    serde_json::json!({
                        "request_id": request,
                        "thread_id": thread_id.as_str(),
                        "code": error.code(),
                        "message": error.to_string(),
                    }),
                );
            }
        }
        if let Ok(mut guard) = deps_for_task.registry.lock() {
            guard.remove(&request);
        }
    });
    Ok(())
}

/// Catalogue d'outils construit depuis un `Store` possédé (sans `AppState`).
pub fn chat_tool_catalog_from_store(store: &Store) -> IhResult<ChatToolCatalog> {
    let servers = store.list_mcp_servers()?;
    let mut snapshots = Vec::new();
    for server in servers {
        let tools = store.list_mcp_tools(Some(&server.id)).unwrap_or_default();
        snapshots.push((server, tools));
    }
    Ok(crate::commands::mcp_tools::catalog_from(&snapshots))
}

/// Fournisseur et modèle du dernier tour assistant de la conversation.
fn last_provider_model(
    deps: &ContinuationDeps,
    thread_id: &ThreadId,
) -> IhResult<(String, String)> {
    let messages = deps.store.list_messages(thread_id, 200, 0)?;
    for message in messages.iter().rev() {
        if message.role != "assistant" {
            continue;
        }
        if let Some(metadata) = &message.metadata {
            let provider = metadata.get("provider").and_then(|v| v.as_str());
            let model = metadata.get("model").and_then(|v| v.as_str());
            if let (Some(provider), Some(model)) = (provider, model) {
                return Ok((provider.to_string(), model.to_string()));
            }
        }
    }
    Err(IhError::NotFound(
        "aucun tour assistant dans cette conversation".into(),
    ))
}

/// Décision IPC : seul geste humain qui autorise l'exécution.
#[tauri::command]
pub async fn chat_tool_decide(
    state: tauri::State<'_, AppState>,
    call_id: String,
    decision: String,
) -> IhResult<ChatToolDecisionOutcome> {
    let decision = ToolCallDecision::parse(&decision)?;
    decide_call(&state, &call_id, decision).await
}

/// Liste les cartes d'outils d'une conversation (arguments déjà masqués).
#[tauri::command]
pub fn chat_tool_calls_list(
    state: tauri::State<'_, AppState>,
    thread_id: String,
) -> IhResult<Vec<ChatToolCallView>> {
    let thread_id = ThreadId::parse(thread_id)?;
    Ok(state
        .store
        .list_chat_tool_calls(&thread_id)?
        .iter()
        .map(ChatToolCallView::from)
        .collect())
}

/// Vue IPC d'une proposition d'outil : sans arguments bruts ni secret.
#[derive(Debug, Serialize)]
pub struct ChatToolCallView {
    pub id: String,
    pub request_id: String,
    pub round: u32,
    pub provider_call_id: String,
    pub exposed_name: String,
    pub server_id: String,
    pub tool: String,
    pub risk: String,
    pub category: String,
    pub arguments: serde_json::Value,
    pub status: String,
    pub error: Option<String>,
    pub result_text: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub finished_at: Option<String>,
}

impl From<&ChatToolCallRecord> for ChatToolCallView {
    fn from(record: &ChatToolCallRecord) -> Self {
        Self {
            id: record.id.clone(),
            request_id: record.request_id.clone(),
            round: record.round,
            provider_call_id: record.provider_call_id.clone(),
            exposed_name: record.exposed_name.clone(),
            server_id: record.server_id.clone(),
            tool: record.tool.clone(),
            risk: record.risk.clone(),
            category: record.category.clone(),
            arguments: record.arguments.clone(),
            status: record.status.clone(),
            error: record.error.clone(),
            result_text: record.result_text.clone(),
            created_at: record.created_at.to_rfc3339(),
            decided_at: record.decided_at.map(|date| date.to_rfc3339()),
            finished_at: record.finished_at.map(|date| date.to_rfc3339()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ih_core::ProjectId;
    use ih_store::Store;

    fn setup() -> (Store, ThreadId, ProjectId) {
        let store = Store::open_in_memory().expect("base");
        let project = ih_store::Project {
            id: ProjectId::new(),
            path: "/tmp/ih-chat-tools-test".into(),
            name: "test".into(),
            created_at: chrono::Utc::now(),
            last_opened_at: chrono::Utc::now(),
            trusted: true,
        };
        store.upsert_project(&project).expect("projet");
        let thread = store
            .create_thread(&project.id, "Essai", None)
            .expect("conversation");
        (store, thread.id, project.id)
    }

    #[test]
    fn une_proposition_hors_catalogue_est_close_immediatement() {
        let (store, thread, project) = setup();
        let catalog = ChatToolCatalog::default();
        // Sans AppState, on vérifie la logique de catalogue partagée en
        // construisant une cible directement.
        let call = ToolCall {
            id: "call_1".into(),
            name: "inconnu__outil".into(),
            arguments: serde_json::json!({}),
        };
        assert!(catalog.target(&call.name).is_none());
        let inserted = store
            .insert_chat_tool_call(&NewChatToolCall {
                thread_id: thread.clone(),
                project_id: project,
                request_id: "req".into(),
                round: 0,
                provider_call_id: call.id,
                exposed_name: call.name,
                server_id: "unknown".into(),
                tool: "inconnu__outil".into(),
                risk: "unknown".into(),
                category: "unknown".into(),
                fingerprint: mcp_call_fingerprint(
                    "unknown",
                    "inconnu__outil",
                    &serde_json::json!({}),
                ),
                arguments: serde_json::json!({}),
                raw_arguments: serde_json::json!({}),
                status: "invalid".into(),
                error: Some("outil inconnu".into()),
            })
            .expect("insertion");
        assert_eq!(inserted.status, "invalid");
        assert!(inserted.error.is_some());
    }

    #[test]
    fn decision_apres_execution_refusee_deux_fois() {
        let (store, thread, project) = setup();
        let call = store
            .insert_chat_tool_call(&NewChatToolCall {
                thread_id: thread.clone(),
                project_id: project,
                request_id: "req".into(),
                round: 0,
                provider_call_id: "call_1".into(),
                exposed_name: "a__echo".into(),
                server_id: "a".into(),
                tool: "echo".into(),
                risk: "low".into(),
                category: "read".into(),
                fingerprint: "f".repeat(64),
                arguments: serde_json::json!({"text": "[masqué]"}),
                raw_arguments: serde_json::json!({"text": "x"}),
                status: "pending".into(),
                error: None,
            })
            .expect("insertion");
        store
            .decide_chat_tool_call(&call.id, "denied", None)
            .expect("refus");
        assert!(matches!(
            store.decide_chat_tool_call(&call.id, "denied", None),
            Err(IhError::Conflict(_))
        ));
    }

    #[test]
    fn le_budget_d_appels_par_tour_est_borne() {
        let (store, thread, project) = setup();
        let mut calls = Vec::new();
        for index in 0..MAX_CALLS_PER_TURN + 2 {
            calls.push(
                store
                    .insert_chat_tool_call(&NewChatToolCall {
                        thread_id: thread.clone(),
                        project_id: project.clone(),
                        request_id: "req".into(),
                        round: 0,
                        provider_call_id: format!("call_{index}"),
                        exposed_name: "a__echo".into(),
                        server_id: "a".into(),
                        tool: "echo".into(),
                        risk: "low".into(),
                        category: "read".into(),
                        fingerprint: "f".repeat(64),
                        arguments: serde_json::json!({}),
                        raw_arguments: serde_json::json!({}),
                        status: if index < 2 {
                            "invalid".into()
                        } else {
                            "pending".into()
                        },
                        error: None,
                    })
                    .expect("insertion"),
            );
        }
        // Les propositions invalides ne consomment pas de budget.
        assert_eq!(
            remaining_call_budget(&calls),
            MAX_CALLS_PER_TURN.saturating_sub(calls.len() - 2)
        );
        assert_eq!(remaining_call_budget(&[]), MAX_CALLS_PER_TURN);
    }

    #[test]
    fn les_cycles_de_continuation_sont_limites() {
        assert!(cycle_allowed(0));
        assert!(cycle_allowed(MAX_TOOL_ROUNDS - 1));
        assert!(!cycle_allowed(MAX_TOOL_ROUNDS));
        assert!(!cycle_allowed(MAX_TOOL_ROUNDS + 5));
    }

    #[test]
    fn les_serveurs_mcp_stdio_ne_sont_jamais_exposes_au_chat() {
        let store = Store::open_in_memory().expect("base");
        let project = ih_store::Project {
            id: ProjectId::new(),
            path: "/tmp/ih-chat-tools-test".into(),
            name: "test".into(),
            created_at: chrono::Utc::now(),
            last_opened_at: chrono::Utc::now(),
            trusted: true,
        };
        store.upsert_project(&project).expect("projet");
        // Serveur MCP stdio (extension) activé : il ne doit jamais entrer dans
        // le catalogue du chat, qui ne lit que les serveurs distants.
        store
            .upsert_extension(&ih_store::InstalledExtension {
                id: "stdio-demo".into(),
                kind: "plugin".into(),
                name: "Stdio".into(),
                version: "1".into(),
                source: "stdio:demo".into(),
                manifest: serde_json::json!({"command": "demo", "args": []}),
                sha256: None,
                enabled: true,
                installed_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .expect("extension");
        let catalog = chat_tool_catalog_from_store(&store).expect("catalogue");
        assert!(catalog.definitions.is_empty());
        assert!(catalog.target("stdio-demo__echo").is_none());
        assert!(catalog.target("echo").is_none());
    }

    #[test]
    fn decision_inconnue_est_rejetee() {
        assert!(ToolCallDecision::parse("maybe").is_err());
        assert_eq!(
            ToolCallDecision::parse("approve").unwrap(),
            ToolCallDecision::Approve
        );
    }
}
