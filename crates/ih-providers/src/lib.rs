//! Clients réels des fournisseurs IA d'IH.
//!
//! Le crate normalise quatre familles d'API derrière le trait [`Provider`] :
//!
//! - [`OpenAiCompatibleProvider`] : OpenAI, OpenRouter, LM Studio, vLLM, etc.
//!   (`POST {base_url}/chat/completions`) ;
//! - [`AnthropicProvider`] : `POST /v1/messages` (SSE `content_block_delta`) ;
//! - [`GoogleProvider`] : `POST /v1beta/models/{model}:generateContent` et
//!   `:streamGenerateContent?alt=sse` ;
//! - [`OllamaProvider`] : `POST /api/chat` en NDJSON, sans clé.
//!
//! Règles de sécurité : aucun secret n'est journalisé, et une clé absente sur un
//! fournisseur cloud est refusée **avant** toute connexion réseau.
//!
//! Limites actuelles : pas de tool-calling natif, pas d'envoi d'images ni de
//! fichiers, pas de comptage local de jetons. Les capacités annoncées par
//! [`ModelEntry`] ne sont renseignées que si l'API les expose réellement.

mod anthropic;
mod common;
mod google;
mod ollama;
mod openai;
mod stream;

pub use anthropic::AnthropicProvider;
pub use google::GoogleProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiCompatibleProvider;

pub use ih_core::config::{ModelEntry, ProviderConfig, ProviderKind};
pub use ih_core::{IhError, IhResult};

use serde::{Deserialize, Serialize};

/// Définition d'outil transmise à un fournisseur de tool calling natif.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Schéma JSON (objet) des paramètres de l'outil.
    pub parameters: serde_json::Value,
}

/// Demande d'exécution d'outil émise par le modèle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Identifiant d'appel choisi par le fournisseur (associé au résultat).
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// Résultat d'outil renvoyé au fournisseur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub tool_call_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
}

/// Message de conversation, indépendant du fournisseur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Appels d'outils demandés par le modèle (message assistant).
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Identifiant de l'appel auquel ce message `tool` répond.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Requête de conversation normalisée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Outils natifs proposés au modèle (vide : comportement classique).
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

impl ChatRequest {
    /// Requête minimale avec un seul message utilisateur.
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: prompt.into(),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }],
            system: None,
            temperature: None,
            max_tokens: None,
            tools: Vec::new(),
        }
    }
}

/// Consommation de jetons rapportée par le fournisseur (si disponible).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

/// Réponse complète normalisée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub text: String,
    pub usage: Usage,
    pub finish_reason: Option<String>,
    pub model: String,
    /// Appels d'outils demandés par le modèle (vide sinon).
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

/// Résultat d'une vérification de disponibilité.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub ok: bool,
    pub latency_ms: u64,
    pub detail: String,
}

/// Contrat commun à tous les fournisseurs.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// Identifiant de configuration (par exemple `openai`).
    fn id(&self) -> &str;

    /// Famille d'API du fournisseur.
    fn kind(&self) -> ProviderKind;

    /// Le fournisseur annonce-t-il le tool calling natif ?
    ///
    /// Capacité **déclarée** (configuration), jamais devinée : un endpoint
    /// « compatible OpenAI » peut ne pas implémenter les outils. Défaut : non
    /// (le chat garde alors son comportement classique).
    fn supports_native_tools(&self) -> bool {
        false
    }

    /// Modèles réellement exposés par l'API.
    async fn list_models(&self) -> IhResult<Vec<ModelEntry>>;

    /// Vérification légère, chronométrée, sans coût d'inférence.
    async fn health(&self) -> IhResult<HealthReport>;

    /// Conversation non-streaming.
    async fn chat(&self, request: ChatRequest) -> IhResult<ChatResponse>;

    /// Streaming : `on_delta` reçoit chaque fragment dans l'ordre, et la
    /// réponse complète (texte concaténé, usage, raison d'arrêt) est renvoyée.
    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> IhResult<ChatResponse>;
}

/// Classe un échec de connexion en statut lisible pour l'interface.
///
/// Volontairement heuristique : les clients normalisent déjà leurs messages,
/// et cette fonction reste testable sans réseau.
pub fn classify_connection_error(message: &str) -> &'static str {
    let text = message.to_ascii_lowercase();
    if text.contains("timed out") || text.contains("timeout") || text.contains("délai") {
        "timeout"
    } else if text.contains("401")
        || text.contains("403")
        || text.contains("clé")
        || text.contains("api key")
        || text.contains("unauthorized")
        || text.contains("forbidden")
    {
        "auth_failed"
    } else if text.contains("404")
        || text.contains("405")
        || text.contains("html")
        || text.contains("invalide") && text.contains("json")
    {
        "unsupported_protocol"
    } else if text.contains("connect")
        || text.contains("dns")
        || text.contains("refused")
        || text.contains("unreachable")
        || text.contains("joindre")
    {
        "unreachable"
    } else {
        "error"
    }
}

/// Fabrique de fournisseurs.
pub struct ProviderFactory;

impl ProviderFactory {
    /// Construit le fournisseur correspondant au `kind` de la configuration.
    pub fn build(config: &ProviderConfig, secret: Option<String>) -> IhResult<Box<dyn Provider>> {
        build_provider(config, secret)
    }
}

/// Construit un fournisseur à partir de sa configuration et de son secret.
///
/// Une configuration cloud sans secret échoue immédiatement avec
/// [`IhError::Provider`] ; aucune requête anonyme n'est tentée.
pub fn build_provider(
    config: &ProviderConfig,
    secret: Option<String>,
) -> IhResult<Box<dyn Provider>> {
    match config.kind {
        ProviderKind::OpenaiCompatible => {
            Ok(Box::new(OpenAiCompatibleProvider::new(config, secret)?))
        }
        ProviderKind::Anthropic => Ok(Box::new(AnthropicProvider::new(config, secret)?)),
        ProviderKind::Google => Ok(Box::new(GoogleProvider::new(config, secret)?)),
        ProviderKind::Ollama => Ok(Box::new(OllamaProvider::new(config)?)),
    }
}

#[cfg(test)]
mod connection_tests {
    use super::classify_connection_error;

    #[test]
    fn classifie_les_echecs_courants() {
        assert_eq!(
            classify_connection_error("fournisseur perso : délai dépassé"),
            "timeout"
        );
        assert_eq!(
            classify_connection_error("HTTP 401 : clé invalide"),
            "auth_failed"
        );
        assert_eq!(
            classify_connection_error("HTTP 403 forbidden"),
            "auth_failed"
        );
        assert_eq!(
            classify_connection_error("HTTP 404 endpoint introuvable"),
            "unsupported_protocol"
        );
        assert_eq!(
            classify_connection_error("connexion refusée (connect error)"),
            "unreachable"
        );
        assert_eq!(classify_connection_error("bizarre"), "error");
    }
}
