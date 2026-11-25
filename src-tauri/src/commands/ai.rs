//! Commandes agents et fournisseurs IA.

use crate::state::AppState;
use ih_agents::AgentRegistry;
use ih_core::config::{ProviderConfig, ProviderKind};
use ih_core::{IhError, IhResult};
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
    pub custom: bool,
    pub endpoint_path: Option<String>,
    pub headers_count: usize,
    pub streaming: bool,
    /// Capacité tool calling natif déclarée par l'utilisateur.
    pub supports_tools: bool,
    pub timeout_secs: u64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[tauri::command]
pub fn providers_list(state: State<'_, AppState>) -> IhResult<Vec<ProviderView>> {
    providers_list_view(&state)
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

/// Saisie utilisateur pour créer ou modifier un provider personnalisé.
#[derive(Debug, serde::Deserialize)]
pub struct ProviderInput {
    pub id: String,
    pub label: String,
    pub kind: ProviderKind,
    pub base_url: String,
    #[serde(default)]
    pub endpoint_path: Option<String>,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_true")]
    pub streaming: bool,
    /// Déclaration explicite du support du tool calling natif.
    #[serde(default)]
    pub supports_tools: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
    /// Clé d'API transmise une seule fois : stockée dans le trousseau, jamais
    /// dans la configuration ni dans la réponse.
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_timeout() -> u64 {
    120
}
fn default_true() -> bool {
    true
}

fn validate_provider_id(id: &str) -> IhResult<()> {
    let ok = (2..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !ok {
        return Err(IhError::Invalid(
            "identifiant de provider invalide (2–32 caractères : a-z, 0-9, - ou _)".into(),
        ));
    }
    Ok(())
}

fn apply_input(provider: &mut ProviderConfig, input: &ProviderInput) {
    provider.label = input.label.trim().to_string();
    provider.kind = input.kind;
    provider.base_url = input.base_url.trim().to_string();
    provider.endpoint_path = input
        .endpoint_path
        .as_ref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    provider.headers = input
        .headers
        .iter()
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    provider.timeout_secs = input.timeout_secs;
    provider.streaming = input.streaming;
    if let Some(supports_tools) = input.supports_tools {
        provider.supports_tools = supports_tools;
    }
    provider.temperature = input.temperature;
    provider.max_tokens = input.max_tokens;
    provider.default_model = input
        .default_model
        .as_ref()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    provider.models = input.models.clone();
    provider.updated_at = Some(chrono::Utc::now());
}

fn store_secret_if_any(state: &AppState, id: &str, api_key: &Option<String>) -> IhResult<bool> {
    match api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        Some(secret) => {
            state.secrets.set(&provider_account(id), secret)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn find_view(state: &AppState, id: &str) -> IhResult<ProviderView> {
    providers_list_view(state)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| IhError::NotFound(format!("provider {id}")))
}

fn providers_list_view(state: &AppState) -> IhResult<Vec<ProviderView>> {
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
            // Drapeau persisté : ne pas lire le trousseau ici (cette liste est
            // chargée au démarrage et déclencherait une demande de mot de passe).
            secret_stored: p.has_secret,
            default_model: p.default_model.clone(),
            models: p.models.clone(),
            custom: p.custom,
            endpoint_path: p.endpoint_path.clone(),
            headers_count: p.headers.len(),
            streaming: p.streaming,
            supports_tools: p.supports_tools,
            timeout_secs: p.timeout_secs,
            created_at: p.created_at.map(|d| d.to_rfc3339()),
            updated_at: p.updated_at.map(|d| d.to_rfc3339()),
        })
        .collect())
}

#[tauri::command]
pub fn provider_create(state: State<'_, AppState>, input: ProviderInput) -> IhResult<ProviderView> {
    let id = input.id.trim().to_ascii_lowercase();
    validate_provider_id(&id)?;
    if state.config_snapshot()?.provider(&id).is_some() {
        return Err(IhError::Conflict(format!("provider déjà existant : {id}")));
    }
    let mut provider = ProviderConfig {
        id: id.clone(),
        custom: true,
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    };
    apply_input(&mut provider, &input);
    let stored = store_secret_if_any(&state, &id, &input.api_key)?;
    provider.has_secret = stored;

    state.update_config(|config| config.providers.push(provider))?;
    find_view(&state, &id)
}

#[tauri::command]
pub fn provider_update(
    state: State<'_, AppState>,
    id: String,
    input: ProviderInput,
) -> IhResult<ProviderView> {
    let target = id.trim().to_string();
    state
        .config_snapshot()?
        .provider(&target)
        .ok_or_else(|| IhError::NotFound(format!("provider {target}")))?;
    let stored = store_secret_if_any(&state, &target, &input.api_key)?;

    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&target) {
            let created = provider.created_at;
            let custom = provider.custom;
            apply_input(provider, &input);
            provider.created_at = created;
            provider.custom = custom;
            if stored {
                provider.has_secret = true;
            }
        }
    })?;
    find_view(&state, &target)
}

#[tauri::command]
pub fn provider_delete(state: State<'_, AppState>, id: String) -> IhResult<()> {
    let provider = state
        .config_snapshot()?
        .provider(&id)
        .cloned()
        .ok_or_else(|| IhError::NotFound(format!("provider {id}")))?;
    if !provider.custom {
        return Err(IhError::Denied(
            "un provider intégré ne peut pas être supprimé (désactivez-le)".into(),
        ));
    }
    state.secrets.delete(&provider_account(&id))?;
    state.update_config(|config| config.providers.retain(|p| p.id != id))?;
    Ok(())
}

/// Test de connexion : accessibilité, authentification, protocole, modèles.
#[derive(Debug, Serialize)]
pub struct ConnectionTest {
    /// `ok` | `auth_failed` | `unreachable` | `unsupported_protocol`
    /// | `model_unavailable` | `timeout` | `error`
    pub status: String,
    pub detail: String,
    pub latency_ms: u64,
    pub models: usize,
    pub protocol: String,
}

#[tauri::command]
pub async fn provider_test(state: State<'_, AppState>, id: String) -> IhResult<ConnectionTest> {
    let provider = build(&state, &id)?;
    let protocol = format!("{:?}", provider.kind());
    let started = std::time::Instant::now();

    let health = provider.health().await?;
    let latency_ms = started.elapsed().as_millis() as u64;
    if !health.ok {
        return Ok(ConnectionTest {
            status: ih_providers::classify_connection_error(&health.detail).into(),
            detail: health.detail,
            latency_ms,
            models: 0,
            protocol,
        });
    }

    match provider.list_models().await {
        Ok(models) => Ok(ConnectionTest {
            status: "ok".into(),
            detail: format!("connexion établie, {} modèle(s) découvert(s)", models.len()),
            latency_ms,
            models: models.len(),
            protocol,
        }),
        Err(error) => Ok(ConnectionTest {
            status: "model_unavailable".into(),
            detail: format!("connexion établie mais découverte des modèles impossible : {error}"),
            latency_ms,
            models: 0,
            protocol,
        }),
    }
}

#[tauri::command]
pub fn provider_add_model(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
    label: Option<String>,
) -> IhResult<ProviderView> {
    let model_id = model_id.trim().to_string();
    if model_id.is_empty() || model_id.len() > 200 {
        return Err(IhError::Invalid("identifiant de modèle invalide".into()));
    }
    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&provider_id) {
            if !provider.models.iter().any(|m| m.id == model_id) {
                provider.models.push(ModelEntry {
                    label: label.unwrap_or_else(|| model_id.clone()),
                    id: model_id.clone(),
                    local: !provider.kind.requires_secret(),
                    ..Default::default()
                });
            }
            provider.updated_at = Some(chrono::Utc::now());
        }
    })?;
    find_view(&state, &provider_id)
}

#[tauri::command]
pub fn provider_remove_model(
    state: State<'_, AppState>,
    provider_id: String,
    model_id: String,
) -> IhResult<ProviderView> {
    state.update_config(|config| {
        if let Some(provider) = config.provider_mut(&provider_id) {
            provider.models.retain(|m| m.id != model_id);
            if provider.default_model.as_deref() == Some(model_id.as_str()) {
                provider.default_model = None;
            }
            provider.updated_at = Some(chrono::Utc::now());
        }
    })?;
    find_view(&state, &provider_id)
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
