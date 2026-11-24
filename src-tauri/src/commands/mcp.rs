//! Commandes MCP distantes (Streamable HTTP et SSE).
//!
//! Toute exécution passe par cette couche : le chargement du serveur et de
//! l'outil, la validation des arguments, la décision d'approbation liée à une
//! empreinte, la connexion/session, l'appel borné puis l'historisation et la
//! diffusion d'événements. Aucun autre chemin ne peut appeler un outil MCP.

use crate::state::AppState;
use ih_core::{IhError, IhResult};
use ih_mcp::http::build_custom_headers;
use ih_mcp::schema::validate_arguments;
use ih_mcp::{McpSession, McpTransport, ServerConfig};
use ih_security::mcp::{
    decide_mcp_call, mcp_call_fingerprint, mcp_secret_account, McpCallDecision, McpCaller,
};
use ih_security::net::UrlPolicy;
use ih_security::risk::RiskLevel;
use ih_store::{McpServerRecord, McpToolRecord, NewAuditEntry, NewMcpCall};
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, State};

const MAX_ERROR_CHARS: usize = 600;
const MAX_ARGUMENTS_BYTES: usize = 8 * 1024;

// ------------------------------------------------------------------ vues IPC

#[derive(Debug, Serialize)]
pub struct McpServerView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub transport: String,
    pub timeout_secs: u32,
    pub enabled: bool,
    pub allow_auto_approve: bool,
    pub has_secret: bool,
    pub last_status: String,
    pub last_error: Option<String>,
    pub last_connected_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub tool_count: usize,
    pub connected: bool,
}

#[derive(Debug, Serialize)]
pub struct McpToolView {
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
    pub category: String,
    pub risk: String,
    pub enabled: bool,
    pub discovered_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct McpCallOutcome {
    /// `approval_required`, `completed`.
    pub status: String,
    pub approval_id: Option<String>,
    /// Empreinte des paramètres (affichée avec la demande d'approbation).
    pub fingerprint: Option<String>,
    pub result: Option<ih_mcp::ToolCallResult>,
}

#[derive(Debug, Serialize)]
pub struct McpTestOutcome {
    pub ok: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

// ----------------------------------------------------------------- helpers

fn risk_from_str(value: &str) -> IhResult<RiskLevel> {
    match value {
        "low" => Ok(RiskLevel::Low),
        "medium" => Ok(RiskLevel::Medium),
        "high" => Ok(RiskLevel::High),
        "critical" => Ok(RiskLevel::Critical),
        other => Err(IhError::Invalid(format!(
            "risque d'outil MCP inconnu : {other}"
        ))),
    }
}

fn transport_from_str(value: &str) -> IhResult<McpTransport> {
    McpTransport::parse(value)
        .ok_or_else(|| IhError::Invalid(format!("transport MCP inconnu : {value}")))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

fn emit_event(state: &AppState, event: &str, payload: serde_json::Value) {
    let _ = state.app_handle.emit(event, payload);
}

fn server_config(state: &AppState, record: &McpServerRecord) -> IhResult<ServerConfig> {
    let mut config = ServerConfig::new(
        record.id.clone(),
        record.url.clone(),
        transport_from_str(&record.transport)?,
    );
    config.name = record.name.clone();
    config.timeout = Duration::from_secs(u64::from(record.timeout_secs));
    config.headers = record.headers.clone();
    // Jeton lu dans le trousseau, jamais depuis la base.
    config.bearer = state.secrets.get(&mcp_secret_account(&record.id))?;
    config.policy = UrlPolicy::default();
    Ok(config)
}

fn set_status(state: &AppState, id: &str, status: &str, error: Option<&str>) {
    let _ = state.store.set_mcp_server_status(id, status, error, None);
    emit_event(
        state,
        "ih://mcp-status",
        serde_json::json!({ "server_id": id, "status": status, "error": error }),
    );
}

/// Ouvre une session (statut `connecting` → `connected`/`error`).
async fn connect_server(state: &AppState, record: &McpServerRecord) -> IhResult<McpSession> {
    if !record.enabled {
        return Err(IhError::Denied(format!(
            "serveur MCP « {} » désactivé",
            record.name
        )));
    }
    set_status(state, &record.id, "connecting", None);
    let config = server_config(state, record)?;
    match McpSession::connect(config).await {
        Ok(session) => {
            let _ = state.store.set_mcp_server_status(
                &record.id,
                "connected",
                None,
                Some(chrono::Utc::now()),
            );
            emit_event(
                state,
                "ih://mcp-status",
                serde_json::json!({ "server_id": record.id, "status": "connected" }),
            );
            Ok(session)
        }
        Err(error) => {
            let detail = truncate(&error.to_string(), MAX_ERROR_CHARS);
            let _ = state
                .store
                .set_mcp_server_status(&record.id, "error", Some(&detail), None);
            emit_event(
                state,
                "ih://mcp-status",
                serde_json::json!({ "server_id": record.id, "status": "error", "error": detail }),
            );
            Err(error)
        }
    }
}

fn tools_to_records(server_id: &str, tools: &[ih_mcp::ToolInfo]) -> Vec<McpToolRecord> {
    tools
        .iter()
        .map(|tool| McpToolRecord {
            server_id: server_id.to_string(),
            name: tool.name.clone(),
            description: truncate(&tool.description, 2000),
            input_schema: tool.input_schema.clone(),
            output_schema: tool.output_schema.clone(),
            category: tool.safety.category.as_str().to_string(),
            risk: tool.safety.risk.label().to_string(),
            enabled: false,
            discovered_at: chrono::Utc::now(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn record_call(
    state: &AppState,
    server_id: &str,
    tool: &str,
    caller: &str,
    caller_id: Option<&str>,
    arguments: &serde_json::Value,
    status: &str,
    error: Option<String>,
    duration_ms: Option<u64>,
) {
    let mut arguments = arguments.clone();
    if serde_json::to_string(&arguments)
        .map(|text| text.len() > MAX_ARGUMENTS_BYTES)
        .unwrap_or(false)
    {
        arguments = serde_json::json!({ "_tronque": true });
    }
    let result = state.store.record_mcp_call(&NewMcpCall {
        server_id: server_id.to_string(),
        tool_name: tool.to_string(),
        caller: caller.to_string(),
        caller_id: caller_id.map(String::from),
        arguments,
        status: status.to_string(),
        error: error.map(|text| truncate(&text, MAX_ERROR_CHARS)),
        duration_ms,
    });
    if let Err(error) = result {
        tracing::warn!(%error, "historisation d'un appel MCP impossible");
    }
}

fn audit(state: &AppState, summary: String, detail: String, decision: &str) {
    let _ = state.store.append_audit(&NewAuditEntry {
        category: "mcp_tool".into(),
        summary: truncate(&summary, 300),
        detail: Some(truncate(&detail, 2000)),
        decision: decision.to_string(),
    });
}

fn tool_view(record: &McpToolRecord) -> McpToolView {
    McpToolView {
        server_id: record.server_id.clone(),
        name: record.name.clone(),
        description: record.description.clone(),
        input_schema: record.input_schema.clone(),
        output_schema: record.output_schema.clone(),
        category: record.category.clone(),
        risk: record.risk.clone(),
        enabled: record.enabled,
        discovered_at: record.discovered_at,
    }
}

// ------------------------------------------------------------ serveurs MCP

#[tauri::command]
pub async fn mcp_remote_servers_list(state: State<'_, AppState>) -> IhResult<Vec<McpServerView>> {
    let sessions = state
        .mcp_sessions
        .lock()
        .await
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let mut views = Vec::new();
    for record in state.store.list_mcp_servers()? {
        let has_secret = state
            .secrets
            .get(&mcp_secret_account(&record.id))
            .ok()
            .flatten()
            .is_some();
        let tool_count = state
            .store
            .list_mcp_tools(Some(&record.id))
            .map(|tools| tools.len())
            .unwrap_or(0);
        tracing::debug!(server = %record.id, enabled = record.enabled, "serveur MCP listé");
        views.push(McpServerView {
            id: record.id.clone(),
            name: record.name.clone(),
            url: record.url.clone(),
            transport: record.transport.clone(),
            timeout_secs: record.timeout_secs,
            enabled: record.enabled,
            allow_auto_approve: record.allow_auto_approve,
            has_secret,
            last_status: record.last_status.clone(),
            last_error: record.last_error.clone(),
            last_connected_at: record.last_connected_at,
            created_at: record.created_at,
            updated_at: record.updated_at,
            tool_count,
            connected: sessions.contains(&record.id),
        });
    }
    Ok(views)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn mcp_server_upsert(
    state: State<'_, AppState>,
    server_id: Option<String>,
    name: String,
    url: String,
    transport: String,
    headers: Option<BTreeMap<String, String>>,
    timeout_secs: u32,
    enabled: bool,
    allow_auto_approve: bool,
    token: Option<String>,
) -> IhResult<McpServerView> {
    if name.trim().is_empty() {
        return Err(IhError::Invalid("nom de serveur MCP vide".into()));
    }
    // Validation immédiate : URL autorisée, en-têtes non réservés.
    ih_security::net::validate_url(&url, UrlPolicy::default())?;
    let headers = headers.unwrap_or_default();
    let _ = build_custom_headers(&headers)?;
    transport_from_str(&transport)?;
    if timeout_secs == 0 {
        return Err(IhError::Invalid(
            "le délai d'un serveur MCP doit être supérieur à zéro".into(),
        ));
    }

    let (id, created_at) = match &server_id {
        Some(id) => {
            let existing = state
                .store
                .get_mcp_server(id)?
                .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
            (existing.id, existing.created_at)
        }
        None => (format!("mcp-{}", uuid::Uuid::new_v4()), chrono::Utc::now()),
    };

    if let Some(token) = &token {
        if !token.trim().is_empty() {
            state.secrets.set(&mcp_secret_account(&id), token)?;
        }
    }

    // Un serveur déjà connecté est déconnecté si son URL change.
    let previous = state.store.get_mcp_server(&id)?;
    let url_changed = previous
        .as_ref()
        .map(|record| record.url != url)
        .unwrap_or(false);
    if url_changed {
        if let Some(session) = state.mcp_sessions.lock().await.remove(&id) {
            let _ = session.close().await;
        }
    }

    let record = McpServerRecord {
        id: id.clone(),
        name,
        url,
        transport,
        headers,
        timeout_secs,
        enabled,
        allow_auto_approve,
        last_status: prev_status(previous.as_ref(), enabled),
        last_error: previous
            .as_ref()
            .and_then(|record| record.last_error.clone()),
        last_connected_at: previous
            .as_ref()
            .and_then(|record| record.last_connected_at),
        created_at,
        updated_at: chrono::Utc::now(),
    };
    state.store.upsert_mcp_server(&record)?;

    let has_secret = state
        .secrets
        .get(&mcp_secret_account(&id))
        .ok()
        .flatten()
        .is_some();
    Ok(McpServerView {
        id: record.id.clone(),
        name: record.name.clone(),
        url: record.url.clone(),
        transport: record.transport.clone(),
        timeout_secs: record.timeout_secs,
        enabled: record.enabled,
        allow_auto_approve: record.allow_auto_approve,
        has_secret,
        last_status: record.last_status.clone(),
        last_error: record.last_error.clone(),
        last_connected_at: record.last_connected_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
        tool_count: state
            .store
            .list_mcp_tools(Some(&record.id))
            .map(|tools| tools.len())
            .unwrap_or(0),
        connected: false,
    })
}

fn prev_status(previous: Option<&McpServerRecord>, enabled: bool) -> String {
    if !enabled {
        return "disconnected".into();
    }
    previous
        .map(|record| record.last_status.clone())
        .filter(|status| status != "connecting")
        .unwrap_or_else(|| "disconnected".into())
}

#[tauri::command]
pub async fn mcp_server_delete(state: State<'_, AppState>, id: String) -> IhResult<()> {
    if let Some(session) = state.mcp_sessions.lock().await.remove(&id) {
        let _ = session.close().await;
    }
    let _ = state.secrets.delete(&mcp_secret_account(&id));
    state.store.delete_mcp_server(&id)?;
    emit_event(
        &state,
        "ih://mcp-status",
        serde_json::json!({ "server_id": id, "status": "deleted" }),
    );
    Ok(())
}

#[tauri::command]
pub async fn mcp_server_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> IhResult<McpServerRecord> {
    let mut record = state
        .store
        .get_mcp_server(&id)?
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
    record.enabled = enabled;
    record.updated_at = chrono::Utc::now();
    if !enabled {
        record.last_status = "disconnected".into();
        if let Some(session) = state.mcp_sessions.lock().await.remove(&id) {
            let _ = session.close().await;
        }
    }
    state.store.upsert_mcp_server(&record)?;
    set_status(&state, &id, "disconnected", None);
    Ok(record)
}

#[tauri::command]
pub fn mcp_server_set_secret(
    state: State<'_, AppState>,
    id: String,
    secret: String,
) -> IhResult<()> {
    state
        .store
        .get_mcp_server(&id)?
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
    state.secrets.set(&mcp_secret_account(&id), &secret)
}

#[tauri::command]
pub fn mcp_server_clear_secret(state: State<'_, AppState>, id: String) -> IhResult<()> {
    state.secrets.delete(&mcp_secret_account(&id))
}

/// Connexion manuelle : ouvre la session, découvre et met en catalogue les
/// outils, puis renvoie le catalogue.
#[tauri::command]
pub async fn mcp_server_connect(
    state: State<'_, AppState>,
    id: String,
) -> IhResult<Vec<McpToolView>> {
    tracing::debug!(server = %id, "commande mcp_server_connect");
    let record = state
        .store
        .get_mcp_server(&id)?
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
    let mut registry = state.mcp_sessions.lock().await;
    let mut session = match registry.remove(&id) {
        Some(existing) if existing.is_transport_alive() => existing,
        _ => connect_server(&state, &record).await?,
    };
    let tools = session.list_tools().await?;
    let records = tools_to_records(&id, &tools);
    state.store.refresh_mcp_tools(&id, &records)?;
    registry.insert(id.clone(), session);
    drop(registry);
    emit_event(
        &state,
        "ih://mcp-discovered",
        serde_json::json!({ "server_id": id, "tools": records.len() }),
    );
    Ok(state
        .store
        .list_mcp_tools(Some(&id))?
        .iter()
        .map(tool_view)
        .collect())
}

#[tauri::command]
pub async fn mcp_server_disconnect(state: State<'_, AppState>, id: String) -> IhResult<()> {
    if let Some(session) = state.mcp_sessions.lock().await.remove(&id) {
        let _ = session.close().await;
    }
    set_status(&state, &id, "disconnected", None);
    Ok(())
}

#[tauri::command]
pub async fn mcp_server_test(state: State<'_, AppState>, id: String) -> IhResult<McpTestOutcome> {
    let record = state
        .store
        .get_mcp_server(&id)?
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
    match connect_server(&state, &record).await {
        Ok(mut session) => {
            let count = match session.list_tools().await {
                Ok(tools) => tools.len(),
                Err(error) => {
                    let _ = session.close().await;
                    let _ = state.store.set_mcp_server_status(
                        &id,
                        "error",
                        Some(&truncate(&error.to_string(), MAX_ERROR_CHARS)),
                        None,
                    );
                    return Ok(McpTestOutcome {
                        ok: false,
                        tool_count: 0,
                        error: Some(truncate(&error.to_string(), MAX_ERROR_CHARS)),
                    });
                }
            };
            let _ = session.close().await;
            set_status(&state, &id, "disconnected", None);
            Ok(McpTestOutcome {
                ok: true,
                tool_count: count,
                error: None,
            })
        }
        Err(error) => Ok(McpTestOutcome {
            ok: false,
            tool_count: 0,
            error: Some(truncate(&error.to_string(), MAX_ERROR_CHARS)),
        }),
    }
}

#[tauri::command]
pub async fn mcp_server_refresh_tools(
    state: State<'_, AppState>,
    id: String,
) -> IhResult<Vec<McpToolView>> {
    let record = state
        .store
        .get_mcp_server(&id)?
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
    let mut registry = state.mcp_sessions.lock().await;
    let mut session = match registry.remove(&id) {
        Some(existing) if existing.is_transport_alive() => existing,
        _ => connect_server(&state, &record).await?,
    };
    let tools = session.list_tools().await?;
    let records = tools_to_records(&id, &tools);
    state.store.refresh_mcp_tools(&id, &records)?;
    registry.insert(id.clone(), session);
    drop(registry);
    emit_event(
        &state,
        "ih://mcp-discovered",
        serde_json::json!({ "server_id": id, "tools": records.len() }),
    );
    Ok(state
        .store
        .list_mcp_tools(Some(&id))?
        .iter()
        .map(tool_view)
        .collect())
}

#[tauri::command]
pub fn mcp_tools_list(
    state: State<'_, AppState>,
    server_id: Option<String>,
) -> IhResult<Vec<McpToolView>> {
    Ok(state
        .store
        .list_mcp_tools(server_id.as_deref())?
        .iter()
        .map(tool_view)
        .collect())
}

#[tauri::command]
pub fn mcp_tool_set_enabled(
    state: State<'_, AppState>,
    server_id: String,
    name: String,
    enabled: bool,
) -> IhResult<McpToolView> {
    state
        .store
        .set_mcp_tool_enabled(&server_id, &name, enabled)?;
    let record = state
        .store
        .get_mcp_tool(&server_id, &name)?
        .ok_or_else(|| IhError::NotFound(format!("outil MCP {server_id}/{name}")))?;
    Ok(tool_view(&record))
}

// ------------------------------------------------------------- appel d'outil

/// Demande d'exécution d'un outil MCP (IPC, conversation, automatisations).
pub struct InvokeRequest {
    pub server_id: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub caller: McpCaller,
    pub caller_id: Option<String>,
    pub approval_id: Option<String>,
}

/// Point d'entrée unique d'exécution d'un outil MCP.
///
/// Vérifie le serveur (désactivé → refus), l'outil (désactivé → refus), les
/// arguments (schéma), puis applique la décision : approbation interactive
/// liée à l'empreinte, ou auto-approbation limitée aux lectures pré-autorisées.
/// Utilisé par l'IPC, la conversation et les automatisations.
pub async fn invoke_mcp_tool(state: &AppState, request: InvokeRequest) -> IhResult<McpCallOutcome> {
    let InvokeRequest {
        server_id,
        tool,
        arguments,
        caller,
        caller_id,
        approval_id,
    } = request;

    let server = state
        .store
        .get_mcp_server(&server_id)?
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {server_id}")))?;
    if !server.enabled {
        record_call(
            state,
            &server_id,
            &tool,
            caller.as_str(),
            caller_id.as_deref(),
            &arguments,
            "denied",
            Some("serveur désactivé".into()),
            None,
        );
        return Err(IhError::Denied(format!(
            "le serveur MCP « {} » est désactivé",
            server.name
        )));
    }

    let tool_record = state
        .store
        .get_mcp_tool(&server_id, &tool)?
        .ok_or_else(|| IhError::NotFound(format!("outil MCP {tool} inconnu")))?;
    if !tool_record.enabled {
        record_call(
            state,
            &server_id,
            &tool,
            caller.as_str(),
            caller_id.as_deref(),
            &arguments,
            "denied",
            Some("outil désactivé".into()),
            None,
        );
        return Err(IhError::Denied(format!(
            "l'outil MCP « {tool} » n'est pas activé par l'utilisateur"
        )));
    }
    validate_arguments(&tool_record.input_schema, &arguments)?;
    let risk = risk_from_str(&tool_record.risk)?;
    let fingerprint = mcp_call_fingerprint(&server_id, &tool, &arguments);
    let summary = format!(
        "Appel de l'outil MCP « {tool} » (serveur « {} »)",
        server.name
    );
    let detail = format!(
        "Outil : {tool}\nServeur : {} ({})\nRisque : {}\nEffet : {}\nParamètres : {}",
        server.name,
        server.url,
        tool_record.risk,
        tool_record.category,
        truncate(&serde_json::to_string(&arguments).unwrap_or_default(), 2000)
    );

    match decide_mcp_call(risk, server.allow_auto_approve, caller) {
        McpCallDecision::RequiresApproval => match &approval_id {
            Some(id) => {
                if let Err(error) =
                    state
                        .approvals
                        .consume_with_fingerprint(id, "mcp_tool", &fingerprint)
                {
                    record_call(
                        state,
                        &server_id,
                        &tool,
                        caller.as_str(),
                        caller_id.as_deref(),
                        &arguments,
                        "denied",
                        Some(error.to_string()),
                        None,
                    );
                    return Err(error);
                }
                audit(state, summary.clone(), detail.clone(), "approved");
            }
            None => {
                let request = state.approvals.ask_with_fingerprint(
                    "mcp_tool",
                    &summary,
                    &detail,
                    risk,
                    Some(fingerprint.clone()),
                )?;
                let approval_id = request.id.into_string();
                record_call(
                    state,
                    &server_id,
                    &tool,
                    caller.as_str(),
                    caller_id.as_deref(),
                    &arguments,
                    "approval_required",
                    None,
                    None,
                );
                emit_event(
                    state,
                    "ih://mcp-call",
                    serde_json::json!({
                        "server_id": server_id,
                        "tool": tool,
                        "phase": "approval_required",
                        "approval_id": approval_id,
                    }),
                );
                return Ok(McpCallOutcome {
                    status: "approval_required".into(),
                    approval_id: Some(approval_id),
                    fingerprint: Some(fingerprint),
                    result: None,
                });
            }
        },
        McpCallDecision::AutoApproved => {
            audit(state, summary.clone(), detail.clone(), "auto");
        }
    }

    // Exécution : session réutilisée ou ouverte à la demande.
    let started = Instant::now();
    emit_event(
        state,
        "ih://mcp-call",
        serde_json::json!({ "server_id": server_id, "tool": tool, "phase": "started" }),
    );
    let mut session = {
        let mut registry = state.mcp_sessions.lock().await;
        match registry.remove(&server_id) {
            Some(existing) if existing.is_transport_alive() => existing,
            _ => connect_server(state, &server).await?,
        }
    };
    let result = session.call_tool(&tool, arguments.clone()).await;
    let alive = session.is_transport_alive();
    if alive {
        state
            .mcp_sessions
            .lock()
            .await
            .insert(server_id.clone(), session);
    } else {
        let _ = session.close().await;
    }
    let duration_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(result) => {
            record_call(
                state,
                &server_id,
                &tool,
                caller.as_str(),
                caller_id.as_deref(),
                &arguments,
                "completed",
                None,
                Some(duration_ms),
            );
            emit_event(
                state,
                "ih://mcp-call",
                serde_json::json!({
                    "server_id": server_id,
                    "tool": tool,
                    "phase": "finished",
                    "status": "completed",
                    "duration_ms": duration_ms,
                }),
            );
            Ok(McpCallOutcome {
                status: "completed".into(),
                approval_id: None,
                fingerprint: None,
                result: Some(result),
            })
        }
        Err(error) => {
            let detail = error.to_string();
            record_call(
                state,
                &server_id,
                &tool,
                caller.as_str(),
                caller_id.as_deref(),
                &arguments,
                "failed",
                Some(detail.clone()),
                Some(duration_ms),
            );
            emit_event(
                state,
                "ih://mcp-call",
                serde_json::json!({
                    "server_id": server_id,
                    "tool": tool,
                    "phase": "finished",
                    "status": "failed",
                    "duration_ms": duration_ms,
                }),
            );
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn mcp_call(
    state: State<'_, AppState>,
    server_id: String,
    tool: String,
    arguments: serde_json::Value,
    caller: Option<String>,
    caller_id: Option<String>,
    approval_id: Option<String>,
) -> IhResult<McpCallOutcome> {
    let caller = caller
        .as_deref()
        .and_then(McpCaller::parse)
        .unwrap_or(McpCaller::Ui);
    invoke_mcp_tool(
        &state,
        InvokeRequest {
            server_id,
            tool,
            arguments,
            caller,
            caller_id,
            approval_id,
        },
    )
    .await
}

/// Appel MCP synchrone pour les contextes non asynchrones (workflows).
///
/// L'appel est exécuté sur un thread dédié avec blocage sur le runtime Tauri :
/// jamais de `block_on` depuis un contexte asynchrone, donc ni blocage du
/// runtime ni de l'interface.
pub fn invoke_mcp_tool_blocking(
    state: &AppState,
    request: InvokeRequest,
) -> IhResult<McpCallOutcome> {
    let app = state.app_handle.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = tauri::async_runtime::block_on(async move {
            let state = app.state::<AppState>();
            invoke_mcp_tool(&state, request).await
        });
        let _ = sender.send(outcome);
    });
    receiver
        .recv()
        .map_err(|_| IhError::Internal("thread d'appel MCP interrompu".into()))?
}

#[tauri::command]
pub fn mcp_calls_list(
    state: State<'_, AppState>,
    limit: Option<u32>,
    server_id: Option<String>,
) -> IhResult<Vec<ih_store::McpCallRecord>> {
    state
        .store
        .list_mcp_calls(limit.unwrap_or(50).min(500), server_id.as_deref())
}

/// Exporte la configuration MCP des serveurs activés (sans aucun secret).
///
/// Destiné aux CLI d'agents qui savent lire une configuration MCP : le jeton
/// reste au trousseau et n'est jamais exporté.
#[tauri::command]
pub fn mcp_export_config(state: State<'_, AppState>) -> IhResult<String> {
    let mut servers = serde_json::Map::new();
    for record in state.store.list_mcp_servers()? {
        if !record.enabled {
            continue;
        }
        let headers: serde_json::Map<String, serde_json::Value> = record
            .headers
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect();
        servers.insert(
            record.id.clone(),
            serde_json::json!({
                "name": record.name,
                "url": record.url,
                "transport": record.transport,
                "headers": headers,
                "auth": if state
                    .secrets
                    .get(&mcp_secret_account(&record.id))
                    .ok()
                    .flatten()
                    .is_some()
                {
                    "bearer (trousseau, non exporté)"
                } else {
                    "none"
                },
            }),
        );
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": servers,
        "note": "Secrets exclus : configurez le jeton dans IH ou dans le CLI lui-même."
    }))
    .unwrap_or_else(|_| "{}".into()))
}
