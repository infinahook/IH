//! Commandes skills et plugins MCP.

use crate::state::AppState;
use ih_core::{IhError, IhResult};
use ih_extensions::mcp::{McpClient, McpServerConfig, McpTool};
use ih_extensions::skills::{install_skill, remove_skill, verify_installed};
use ih_store::InstalledExtension;
use serde::Serialize;
use std::collections::BTreeMap;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct SkillView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub enabled: bool,
    pub sha256: Option<String>,
    pub verified: Option<bool>,
    pub dir: String,
}

#[tauri::command]
pub fn skills_list(state: State<'_, AppState>) -> IhResult<Vec<SkillView>> {
    let installed = state.store.list_extensions("skill")?;
    Ok(installed
        .into_iter()
        .map(|ext| {
            let dir = std::path::Path::new(&ext.source);
            let verified = match &ext.sha256 {
                Some(hash) if dir.is_dir() => verify_installed(dir, hash).ok(),
                _ => None,
            };
            SkillView {
                id: ext.id,
                name: ext.name,
                version: ext.version,
                source: ext.source.clone(),
                enabled: ext.enabled,
                sha256: ext.sha256,
                verified,
                dir: ext.source,
            }
        })
        .collect())
}

#[derive(Debug, Serialize)]
pub struct InstallReportView {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub files: usize,
}

#[tauri::command]
pub fn skill_install(state: State<'_, AppState>, path: String) -> IhResult<InstallReportView> {
    let source = std::path::Path::new(&path);
    if !source.is_dir() {
        return Err(IhError::Invalid(format!(
            "dossier de skill introuvable : {path}"
        )));
    }
    let report = install_skill(source, &state.paths.skills_dir())?;
    let skill_dir = state
        .paths
        .skills_dir()
        .join(format!("{}-{}", report.id, report.version));
    let manifest = ih_extensions::skills::load_manifest(&skill_dir)?;
    let manifest_value = serde_json::to_value(&manifest)?;
    state.store.upsert_extension(&InstalledExtension {
        id: report.id.clone(),
        kind: "skill".into(),
        name: manifest.name.clone(),
        version: report.version.clone(),
        source: skill_dir.display().to_string(),
        manifest: manifest_value,
        sha256: Some(report.sha256.clone()),
        enabled: true,
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    })?;
    Ok(InstallReportView {
        id: report.id,
        version: report.version,
        sha256: report.sha256,
        files: report.files,
    })
}

#[tauri::command]
pub fn skill_verify(state: State<'_, AppState>, id: String) -> IhResult<bool> {
    let extension = state
        .store
        .list_extensions("skill")?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| IhError::NotFound(format!("skill {id}")))?;
    let hash = extension
        .sha256
        .ok_or_else(|| IhError::NotFound("empreinte inconnue".into()))?;
    verify_installed(std::path::Path::new(&extension.source), &hash)
}

#[tauri::command]
pub fn skill_remove(state: State<'_, AppState>, id: String, version: String) -> IhResult<()> {
    remove_skill(&state.paths.skills_dir(), &id, &version)?;
    state.store.remove_extension(&id, "skill")
}

#[tauri::command]
pub fn skill_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> IhResult<()> {
    let mut extension = state
        .store
        .list_extensions("skill")?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| IhError::NotFound(format!("skill {id}")))?;
    extension.enabled = enabled;
    extension.updated_at = chrono::Utc::now();
    state.store.upsert_extension(&extension)
}

// --------------------------------------------------------------------- MCP

#[tauri::command]
pub fn mcp_servers_list(state: State<'_, AppState>) -> IhResult<Vec<InstalledExtension>> {
    state.store.list_extensions("plugin")
}

#[tauri::command]
pub fn mcp_server_add(
    state: State<'_, AppState>,
    id: String,
    name: String,
    command: String,
    args: Vec<String>,
) -> IhResult<()> {
    if id.trim().is_empty() || command.trim().is_empty() {
        return Err(IhError::Invalid(
            "un serveur MCP exige un identifiant et une commande".into(),
        ));
    }
    if !ih_core::platform::find_in_path(&command).is_some() {
        return Err(IhError::Invalid(format!(
            "commande introuvable dans le PATH : {command}"
        )));
    }
    state.store.upsert_extension(&InstalledExtension {
        id: id.clone(),
        kind: "plugin".into(),
        name,
        version: "1".into(),
        source: format!("stdio:{command}"),
        manifest: serde_json::json!({ "command": command, "args": args }),
        sha256: None,
        enabled: false,
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    })
}

#[tauri::command]
pub fn mcp_server_remove(state: State<'_, AppState>, id: String) -> IhResult<()> {
    state.mcp.lock().ok().map(|mut map| map.remove(&id));
    state.store.remove_extension(&id, "plugin")
}

fn server_config(state: &AppState, id: &str) -> IhResult<McpServerConfig> {
    let extension = state
        .store
        .list_extensions("plugin")?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))?;
    let command = extension
        .manifest
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IhError::Invalid(format!("serveur {id} sans commande")))?
        .to_string();
    let args = extension
        .manifest
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServerConfig {
        id: id.to_string(),
        command,
        args,
        env: BTreeMap::new(),
    })
}

#[tauri::command]
pub async fn mcp_connect(
    state: State<'_, AppState>,
    id: String,
    client_name: Option<String>,
) -> IhResult<Vec<McpTool>> {
    let config = server_config(&state, &id)?;
    let mut client = McpClient::spawn(&config).await?;
    client
        .initialize(
            &client_name.unwrap_or_else(|| "IH".into()),
            ih_core::APP_VERSION,
        )
        .await?;
    let tools = client.list_tools().await?;
    state
        .mcp
        .lock()
        .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
        .insert(id, client);
    Ok(tools)
}

#[derive(Debug, Serialize)]
pub struct McpCallOutcome {
    pub status: String,
    pub approval_id: Option<String>,
    pub result: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn mcp_call_tool(
    state: State<'_, AppState>,
    id: String,
    tool: String,
    arguments: serde_json::Value,
    approval_id: Option<String>,
) -> IhResult<McpCallOutcome> {
    let category = "mcp_tool";
    if !arguments.is_object() {
        return Err(IhError::Invalid(
            "les arguments d'un outil MCP doivent être un objet JSON".into(),
        ));
    }
    // Même garantie que le client distant : l'approbation est liée à
    // l'empreinte exacte des paramètres et le risque est classé localement
    // (jamais un niveau figé).
    let fingerprint = ih_security::mcp::mcp_call_fingerprint(&id, &tool, &arguments);
    let safety = ih_security::mcp::McpToolSafety::classify(
        &tool,
        "",
        ih_security::mcp::McpToolHints::default(),
    );
    let approval = match approval_id {
        Some(approval) => {
            state
                .approvals
                .consume_with_fingerprint(&approval, category, &fingerprint)?;
            approval
        }
        None => {
            let request = state.approvals.ask_with_fingerprint(
                category,
                &format!("Appel de l'outil MCP « {tool} »"),
                &ih_core::redaction::redact(&serde_json::to_string(&arguments).unwrap_or_default()),
                safety.risk,
                Some(fingerprint),
            )?;
            return Ok(McpCallOutcome {
                status: "approval_required".into(),
                approval_id: Some(request.id.into_string()),
                result: None,
            });
        }
    };
    let _ = approval;

    let mut client = {
        let mut guard = state
            .mcp
            .lock()
            .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?;
        guard
            .remove(&id)
            .ok_or_else(|| IhError::NotFound(format!("serveur MCP non connecté : {id}")))?
    };
    let result = client.call_tool(&tool, arguments).await;
    state
        .mcp
        .lock()
        .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
        .insert(id, client);
    let result = result?;
    Ok(McpCallOutcome {
        status: "ok".into(),
        approval_id: None,
        result: Some(result),
    })
}

#[tauri::command]
pub async fn mcp_disconnect(state: State<'_, AppState>, id: String) -> IhResult<()> {
    let client = state
        .mcp
        .lock()
        .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
        .remove(&id);
    if let Some(client) = client {
        client.shutdown().await?;
    }
    Ok(())
}
