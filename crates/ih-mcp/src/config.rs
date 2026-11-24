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

/// CLI d'agent cible pour la génération de configuration MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliFlavor {
    /// Claude Code (`mcpServers` avec `type: http|sse`).
    Claude,
    /// Gemini CLI (`mcpServers` avec `url`).
    Gemini,
    /// OpenCode (`mcp` avec `type: remote`).
    Opencode,
}

impl CliFlavor {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "gemini" => Some(Self::Gemini),
            "opencode" => Some(Self::Opencode),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Opencode => "opencode",
        }
    }
}

/// Serveur exporté vers un CLI : aucune valeur secrète n'est incluse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliServer {
    pub id: String,
    pub name: String,
    pub url: String,
    pub transport: McpTransport,
    pub headers: std::collections::BTreeMap<String, String>,
    /// Jeton présent au trousseau (signalé, jamais exporté).
    pub has_secret: bool,
}

/// Génère la configuration MCP d'un CLI d'agent.
///
/// Le format exact de chaque CLI a évolué plusieurs fois ; cette génération
/// couvre le sous-ensemble documenté (URL HTTP/SSE, en-têtes non secrets) et
/// **n'injecte jamais** de jeton : le CLI doit configurer lui-même son
/// authentification. Le fichier généré n'est pas écrit par IH dans cette
/// version : l'utilisateur le copie ou le passe en argument au CLI.
pub fn cli_config(flavor: CliFlavor, servers: &[CliServer]) -> String {
    let enabled: Vec<&CliServer> = servers.iter().collect();
    let mut map = serde_json::Map::new();
    match flavor {
        CliFlavor::Claude | CliFlavor::Gemini => {
            for server in &enabled {
                let mut entry = serde_json::Map::new();
                let transport = if server.transport == McpTransport::Sse {
                    "sse"
                } else {
                    "http"
                };
                entry.insert("type".into(), serde_json::Value::String(transport.into()));
                entry.insert("url".into(), serde_json::Value::String(server.url.clone()));
                if !server.headers.is_empty() {
                    let headers = server
                        .headers
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect::<serde_json::Map<_, _>>();
                    entry.insert("headers".into(), serde_json::Value::Object(headers));
                }
                if server.has_secret {
                    entry.insert(
                        "auth".into(),
                        serde_json::Value::String("bearer (trousseau, non exporté)".into()),
                    );
                }
                map.insert(server.id.clone(), serde_json::Value::Object(entry));
            }
            serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": map }))
                .unwrap_or_default()
        }
        CliFlavor::Opencode => {
            for server in &enabled {
                let mut entry = serde_json::Map::new();
                entry.insert("type".into(), serde_json::Value::String("remote".into()));
                entry.insert("url".into(), serde_json::Value::String(server.url.clone()));
                if !server.headers.is_empty() {
                    let headers = server
                        .headers
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect::<serde_json::Map<_, _>>();
                    entry.insert("headers".into(), serde_json::Value::Object(headers));
                }
                map.insert(server.id.clone(), serde_json::Value::Object(entry));
            }
            serde_json::to_string_pretty(&serde_json::json!({ "mcp": map })).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    fn server(id: &str, transport: McpTransport, secret: bool) -> CliServer {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert("X-Tenant".into(), "ih".into());
        CliServer {
            id: id.into(),
            name: format!("serveur {id}"),
            url: format!("https://mcp.exemple.com/{id}"),
            transport,
            headers,
            has_secret: secret,
        }
    }

    #[test]
    fn configuration_claude_json() {
        let config = cli_config(
            CliFlavor::Claude,
            &[server("a", McpTransport::StreamableHttp, true)],
        );
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(parsed["mcpServers"]["a"]["type"], "http");
        assert_eq!(
            parsed["mcpServers"]["a"]["url"],
            "https://mcp.exemple.com/a"
        );
        assert_eq!(parsed["mcpServers"]["a"]["headers"]["X-Tenant"], "ih");
        assert!(parsed["mcpServers"]["a"]["auth"].is_string());
        // Aucun secret en clair : seul un marqueur est exporté.
        assert!(!config.contains("Bearer "));
    }

    #[test]
    fn transport_sse_marque_sse() {
        let config = cli_config(CliFlavor::Claude, &[server("b", McpTransport::Sse, false)]);
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(parsed["mcpServers"]["b"]["type"], "sse");
    }

    #[test]
    fn configuration_opencode_remote() {
        let config = cli_config(
            CliFlavor::Opencode,
            &[server("c", McpTransport::StreamableHttp, false)],
        );
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(parsed["mcp"]["c"]["type"], "remote");
        assert_eq!(parsed["mcp"]["c"]["url"], "https://mcp.exemple.com/c");
    }

    #[test]
    fn liste_vide_donne_un_objet_vide() {
        let config = cli_config(CliFlavor::Gemini, &[]);
        let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert!(parsed["mcpServers"].as_object().unwrap().is_empty());
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
