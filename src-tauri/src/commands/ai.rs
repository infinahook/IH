//! Commandes agents et fournisseurs IA.

use super::project_path;
use crate::state::AppState;
use ih_agents::{AgentRegistry, StartRequest};
use ih_core::events::AgentState;
use ih_core::{IhError, IhResult, SessionId};
use ih_providers::{build_provider, HealthReport, ModelEntry};
use ih_security::secrets::provider_account;
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub fn agents_list(state: State<'_, AppState>) -> IhResult<Vec<ih_agents::AgentInfo>> {
    let config = state.config_snapshot()?;
    Ok(AgentRegistry::from_configs(&config.agents).list())
}

#[tauri::command]
pub fn agent_start(
    state: State<'_, AppState>,
    project_id: String,
    agent_id: String,
    prompt: String,
) -> IhResult<String> {
    let config = state.config_snapshot()?;
    let agent_config = config
        .agent(&agent_id)
        .ok_or_else(|| IhError::NotFound(format!("agent {agent_id}")))?
        .clone();
    let root = project_path(&state, &project_id)?;
    let adapter = ih_agents::AgentAdapter::from_config(&agent_config);
    let session_id = SessionId::new();
    let session = state.store.create_session(&ih_store::NewSession {
        project_id: ih_core::ProjectId::parse(project_id)?,
        agent_id: agent_id.clone(),
        thread_id: None,
        prompt: prompt.clone(),
    })?;
    let id = state.supervisor.start(StartRequest {
        agent: adapter,
        project_dir: root,
        prompt,
        session_id: Some(session_id.clone()),
    })?;
    state
        .store
        .update_session_state(&session.id, AgentState::Running, None)?;
    Ok(id)
}

#[tauri::command]
pub fn agent_cancel(state: State<'_, AppState>, session_id: String) -> IhResult<()> {
    state.supervisor.cancel(&session_id)
}

#[tauri::command]
pub fn agent_state(
    state: State<'_, AppState>,
    session_id: String,
) -> IhResult<Option<ih_agents::SessionSnapshot>> {
    state.supervisor.snapshot(&session_id)
}

#[tauri::command]
pub fn agent_sessions(
    state: State<'_, AppState>,
    project_id: String,
) -> IhResult<Vec<ih_store::SessionRow>> {
    let id = ih_core::ProjectId::parse(project_id)?;
    state.store.list_sessions(&id, 50)
}

// ------------------------------------------------------------- fournisseurs

#[derive(Debug, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub label: String,
    pub kind: ih_core::config::ProviderKind,
    pub base_url: String,
    pub enabled: bool,
    pub has_secret: bool,
    pub secret_stored: bool,
    pub default_model: Option<String>,
    pub models: Vec<ModelEntry>,
}

#[tauri::command]
pub fn providers_list(state: State<'_, AppState>) -> IhResult<Vec<ProviderView>> {
    let config = state.config_snapshot()?;
    Ok(config
        .providers
        .iter()
        .map(|p| ProviderView {
            id: p.id.clone(),
            label: p.label.clone(),
            kind: p.kind,
            base_url: p.base_url.clone(),
            enabled: p.enabled,
            has_secret: p.has_secret,
            secret_stored: state
                .secrets
                .get(&provider_account(&p.id))
                .ok()
                .flatten()
                .is_some(),
            default_model: p.default_model.clone(),
            models: p.models.clone(),
        })
        .collect())
}

#[tauri::command]
pub fn provider_set_secret(
    state: State<'_, AppState>,
    provider_id: String,
    secret: String,
) -> IhResult<()> {
    state
        .config_snapshot()?
        .provider(&provider_id)
        .ok_or_else(|| IhError::NotFound(format!("fournisseur {provider_id}")))?;
    state
        .secrets
        .set(&provider_account(&provider_id), &secret)?;
    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&provider_id) {
            provider.has_secret = true;
        }
    })?;
    Ok(())
}

#[tauri::command]
pub fn provider_clear_secret(state: State<'_, AppState>, provider_id: String) -> IhResult<()> {
    state.secrets.delete(&provider_account(&provider_id))?;
    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&provider_id) {
            provider.has_secret = false;
        }
    })?;
    Ok(())
}

fn build(state: &AppState, provider_id: &str) -> IhResult<Box<dyn ih_providers::Provider>> {
    let config = state.config_snapshot()?;
    let provider = config
        .provider(provider_id)
        .ok_or_else(|| IhError::NotFound(format!("fournisseur {provider_id}")))?
        .clone();
    let secret = state.secrets.get(&provider_account(provider_id))?;
    build_provider(&provider, secret)
}

#[tauri::command]
pub async fn provider_health(
    state: State<'_, AppState>,
    provider_id: String,
) -> IhResult<HealthReport> {
    let provider = build(&state, &provider_id)?;
    provider.health().await
}

#[tauri::command]
pub async fn models_list(
    state: State<'_, AppState>,
    provider_id: String,
) -> IhResult<Vec<ModelEntry>> {
    let provider = build(&state, &provider_id)?;
    let models = provider.list_models().await?;
    let models_clone = models.clone();
    state.update_config(|config| {
        if let Some(entry) = config.provider_mut(&provider_id) {
            entry.models = models_clone;
        }
    })?;
    Ok(models)
}

#[tauri::command]
pub fn provider_set_default_model(
    state: State<'_, AppState>,
    provider_id: String,
    model: String,
) -> IhResult<()> {
    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&provider_id) {
            provider.default_model = Some(model);
        }
    })?;
    Ok(())
}

#[tauri::command]
pub fn provider_set_enabled(
    state: State<'_, AppState>,
    provider_id: String,
    enabled: bool,
) -> IhResult<()> {
    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&provider_id) {
            provider.enabled = enabled;
        }
    })?;
    Ok(())
}
