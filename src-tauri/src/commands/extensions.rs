//! Commandes skills et plugins MCP.

use crate::commands::mcp::truncate;
use crate::state::AppState;
use ih_core::{IhError, IhResult};
use ih_extensions::mcp::{McpClient, McpServerConfig, McpTool};
use ih_extensions::skills::{install_skill, remove_skill, verify_installed};
use ih_security::mcp::{McpToolHints, McpToolSafety};
use ih_store::{InstalledExtension, NewAuditEntry};
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
        // L'ajout est une action explicite de l'utilisateur : le serveur est
        // actif, mais la connexion et chaque appel restent contrôlés.
        enabled: true,
        installed_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    })
}

#[tauri::command]
pub fn mcp_server_remove(state: State<'_, AppState>, id: String) -> IhResult<()> {
    state.mcp.lock().ok().map(|mut map| map.remove(&id));
    state.store.remove_extension(&id, "plugin")
}

/// Serveur MCP stdio enregistré (extension de type `plugin`).
fn stdio_extension(state: &AppState, id: &str) -> IhResult<InstalledExtension> {
    state
        .store
        .list_extensions("plugin")?
        .into_iter()
        .find(|extension| extension.id == id)
        .ok_or_else(|| IhError::NotFound(format!("serveur MCP {id}")))
}

/// Contrôle d'activation : un serveur désactivé ne connecte ni n'exécute rien.
fn require_stdio_enabled(extension: &InstalledExtension) -> IhResult<()> {
    if extension.enabled {
        return Ok(());
    }
    Err(IhError::Denied(format!(
        "le serveur MCP stdio « {} » est désactivé",
        extension.name
    )))
}

/// Retrouve l'outil dans le catalogue du serveur et valide ses arguments
/// contre le schéma réellement annoncé (jamais un schéma figé).
fn checked_stdio_tool<'a>(
    tools: &'a [McpTool],
    tool_name: &str,
    arguments: &serde_json::Value,
) -> IhResult<(&'a McpTool, McpToolSafety)> {
    let tool = tools
        .iter()
        .find(|candidate| candidate.name == tool_name)
        .ok_or_else(|| {
            IhError::NotFound(format!("outil MCP inconnu sur ce serveur : {tool_name}"))
        })?;
    ih_mcp::schema::validate_arguments(&tool.input_schema, arguments)?;
    let safety = McpToolSafety::classify(&tool.name, &tool.description, McpToolHints::default());
    Ok((tool, safety))
}

/// Active ou désactive un serveur MCP stdio (même contrôle que le distant :
/// serveur activé ET connecté pour appeler un outil).
#[tauri::command]
pub async fn mcp_stdio_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> IhResult<()> {
    let mut extension = stdio_extension(&state, &id)?;
    extension.enabled = enabled;
    extension.updated_at = chrono::Utc::now();
    state.store.upsert_extension(&extension)?;
    if !enabled {
        let client = state
            .mcp
            .lock()
            .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
            .remove(&id);
        if let Some(client) = client {
            // Déconnecter réellement : la session ne doit pas survivre à la
            // désactivation du serveur.
            if let Err(error) = client.shutdown().await {
                tracing::warn!(%error, server = %id, "arrêt du serveur MCP stdio impossible");
            }
        }
    }
    Ok(())
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
    require_stdio_enabled(&stdio_extension(&state, &id)?)?;
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
    // Mêmes contrôles que le garde-fou distant, adaptés au transport stdio :
    // serveur activé, connecté, outil réellement annoncé, arguments validés
    // contre son schéma, classification locale du risque, approbation à usage
    // unique liée à l'empreinte exacte, trace d'audit.
    let extension = stdio_extension(&state, &id)?;
    require_stdio_enabled(&extension)?;

    let mut client = {
        let mut guard = state
            .mcp
            .lock()
            .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?;
        guard
            .remove(&id)
            .ok_or_else(|| IhError::NotFound(format!("serveur MCP non connecté : {id}")))?
    };

    let tools = match client.list_tools().await {
        Ok(tools) => tools,
        Err(error) => {
            state
                .mcp
                .lock()
                .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
                .insert(id, client);
            return Err(error);
        }
    };
    let checked = checked_stdio_tool(&tools, &tool, &arguments);
    let (tool_description, safety) = match checked {
        Ok((tool_info, safety)) => (tool_info.description.clone(), safety),
        Err(error) => {
            state
                .mcp
                .lock()
                .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
                .insert(id, client);
            return Err(error);
        }
    };

    let category = "mcp_tool";
    let fingerprint = ih_security::mcp::mcp_call_fingerprint(&id, &tool, &arguments);
    let summary = format!(
        "Appel de l'outil MCP stdio « {tool} » (serveur « {} »)",
        extension.name
    );
    let detail = format!(
        "Outil : {tool}\nServeur : {} (stdio)\nTransport : stdio\nRisque : {}\nEffet : {}\nDescription : {}\nParamètres : {}",
        extension.name,
        safety.risk.label(),
        safety.category.as_str(),
        truncate(&tool_description, 300),
        truncate(
            &serde_json::to_string(&ih_security::mcp::redact_arguments(&arguments))
                .unwrap_or_default(),
            2000
        )
    );

    // Aucune pré-autorisation n'existe pour un serveur stdio : tout appel
    // exige une approbation interactive à usage unique.
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
                &summary,
                &detail,
                safety.risk,
                Some(fingerprint),
            )?;
            state
                .mcp
                .lock()
                .map_err(|_| IhError::Internal("registre MCP verrouillé".into()))?
                .insert(id, client);
            return Ok(McpCallOutcome {
                status: "approval_required".into(),
                approval_id: Some(request.id.into_string()),
                result: None,
            });
        }
    };
    let _ = approval;

    let _ = state.store.append_audit(&NewAuditEntry {
        category: category.into(),
        summary: truncate(&summary, 300),
        detail: Some(truncate(&detail, 2000)),
        decision: "approved".into(),
    });

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

#[cfg(test)]
mod stdio_tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, description: &str, schema: serde_json::Value) -> McpTool {
        McpTool {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
        }
    }

    #[test]
    fn arguments_conformes_au_schema_acceptes() {
        let tools = vec![tool(
            "echo",
            "Renvoie le texte.",
            json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        )];
        let (found, safety) =
            checked_stdio_tool(&tools, "echo", &json!({"text": "bonjour"})).expect("valide");
        assert_eq!(found.name, "echo");
        // Sans texte d'activation dans le protocole stdio historique, un outil
        // non destructif reste classé « medium » (jamais « low » implicite).
        assert_eq!(safety.risk.label(), "medium");
    }

    #[test]
    fn arguments_hors_schema_refuses() {
        let tools = vec![tool(
            "echo",
            "Renvoie le texte.",
            json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
        )];
        let error = checked_stdio_tool(&tools, "echo", &json!({})).expect_err("champ requis");
        assert!(format!("{error}").contains("text"));
    }

    #[test]
    fn outil_inconnu_refuse() {
        let tools = vec![tool("echo", "", json!({"type": "object"}))];
        let error = checked_stdio_tool(&tools, "delete", &json!({})).expect_err("inconnu");
        assert!(matches!(error, IhError::NotFound(_)));
    }

    #[test]
    fn arguments_non_objet_refuses() {
        let tools = vec![tool("echo", "", json!({"type": "object"}))];
        assert!(checked_stdio_tool(&tools, "echo", &json!(["pas", "objet"])).is_err());
    }

    #[test]
    fn outil_destructif_classe_critique_par_son_nom() {
        let tools = vec![tool(
            "delete_everything",
            "Supprime des données.",
            json!({"type": "object"}),
        )];
        let (_, safety) =
            checked_stdio_tool(&tools, "delete_everything", &json!({})).expect("classé");
        assert_eq!(safety.risk.label(), "critical");
        assert!(!safety.enabled_by_default);
    }

    #[test]
    fn un_serveur_desactive_ne_peut_nie_connecter_nie_appeler() {
        let mut extension = InstalledExtension {
            id: "stdio".into(),
            kind: "plugin".into(),
            name: "Stdio".into(),
            version: "1".into(),
            source: "stdio:demo".into(),
            manifest: json!({"command": "demo", "args": []}),
            sha256: None,
            enabled: false,
            installed_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(matches!(
            require_stdio_enabled(&extension),
            Err(IhError::Denied(_))
        ));
        extension.enabled = true;
        assert!(require_stdio_enabled(&extension).is_ok());
    }
}
