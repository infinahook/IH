//! Configuration d'un serveur MCP distant.

use ih_security::net::UrlPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Transport d'un serveur MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// Transport « Streamable HTTP » (spécification 2025-03-26 et suivantes),
    /// géré par le SDK officiel `rmcp`.
    StreamableHttp,
    /// Transport HTTP+SSE historique (spécification 2024-11-05).
    Sse,
}

impl McpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StreamableHttp => "streamable_http",
            Self::Sse => "sse",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "streamable_http" | "streamable-http" => Some(Self::StreamableHttp),
            "sse" => Some(Self::Sse),
            _ => None,
        }
    }
}

/// Délai par défaut d'une requête MCP.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Taille maximale d'une réponse JSON-RPC (8 Mio, alignée sur le client stdio).
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Taille maximale d'un événement SSE (un message JSON-RPC complet).
pub const MAX_SSE_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Nombre maximal d'outils acceptés à la découverte (protection mémoire).
pub const MAX_TOOLS: usize = 512;

/// Configuration d'une connexion à un serveur MCP distant.
///
/// Le secret (`bearer`) est fourni **en mémoire** par l'appelant, lu dans le
/// trousseau ; il n'est jamais sérialisé ni journalisé par cette crate.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Identifiant local (logs, base de données, empreintes d'approbation).
    pub id: String,
    /// Nom affiché.
    pub name: String,
    pub url: String,
    pub transport: McpTransport,
    pub timeout: Duration,
    /// En-têtes personnalisés non secrets (les clés sensibles sont refusées).
    pub headers: BTreeMap<String, String>,
    /// Jeton bearer, déjà lu depuis le trousseau par l'appelant.
    pub bearer: Option<String>,
    pub policy: UrlPolicy,
}

impl ServerConfig {
    /// Configuration minimale avec le délai par défaut et la politique par défaut.
    pub fn new(id: impl Into<String>, url: impl Into<String>, transport: McpTransport) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            url: url.into(),
            transport,
            timeout: DEFAULT_TIMEOUT,
            headers: BTreeMap::new(),
            bearer: None,
            policy: UrlPolicy::default(),
        }
    }

    /// Vérifie les champs locaux avant toute connexion réseau.
    pub fn validate(&self) -> ih_core::IhResult<()> {
        if self.id.trim().is_empty() {
            return Err(ih_core::IhError::Invalid(
                "identifiant de serveur MCP vide".into(),
            ));
        }
        if self.timeout.is_zero() {
            return Err(ih_core::IhError::Invalid(
                "le délai des requêtes MCP doit être supérieur à zéro".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_aller_retour() {
        for transport in [McpTransport::StreamableHttp, McpTransport::Sse] {
            assert_eq!(McpTransport::parse(transport.as_str()), Some(transport));
        }
        assert_eq!(
            McpTransport::parse("streamable-http"),
            Some(McpTransport::StreamableHttp)
        );
        assert_eq!(McpTransport::parse("websocket"), None);
    }

    #[test]
    fn validation_identifiant_et_delai() {
        let mut config =
            ServerConfig::new("", "https://exemple.com/mcp", McpTransport::StreamableHttp);
        assert!(config.validate().is_err());
        config.id = "srv".into();
        assert!(config.validate().is_ok());
        config.timeout = Duration::ZERO;
        assert!(config.validate().is_err());
    }
}
