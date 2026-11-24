//! Session MCP unifiée : un type commun pour les deux transports réseau.

use crate::config::{McpTransport, ServerConfig};
use crate::sse::SseSession;
use crate::streamable::StreamableSession;
use ih_core::{IhError, IhResult};
use ih_security::mcp::McpToolSafety;
use serde_json::Value;

/// Taille maximale du texte d'un résultat conservé pour l'affichage et la
/// conversation (le reste est signalé tronqué, jamais silencieusement coupé).
pub const MAX_RESULT_TEXT_BYTES: usize = 256 * 1024;

/// Outil découvert sur un serveur MCP.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Schéma JSON d'entrée validé structurellement.
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Classification de risque calculée localement (jamais fournie par le serveur).
    pub safety: McpToolSafety,
}

/// Résultat borné d'un appel d'outil.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolCallResult {
    /// Blocs de contenu bruts renvoyés par le serveur (données non fiables).
    pub content: Vec<Value>,
    /// Texte concaténé des blocs texte, tronqué à [`MAX_RESULT_TEXT_BYTES`].
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    pub is_error: bool,
    /// Vrai si le texte a été tronqué pour l'affichage.
    pub truncated: bool,
}

/// Convertit un résultat `rmcp` en résultat borné d'IH.
pub(crate) fn convert_call_result(
    result: rmcp::model::CallToolResult,
    tool: &str,
) -> ToolCallResult {
    let content = result
        .content
        .iter()
        .map(|block| serde_json::to_value(block).unwrap_or(Value::Null))
        .collect::<Vec<_>>();

    let mut text_parts = Vec::new();
    for block in &result.content {
        if let rmcp::model::ContentBlock::Text(text) = block {
            text_parts.push(text.text.clone());
        }
    }
    let mut text = text_parts.join("\n");
    let mut truncated = false;
    if text.len() > MAX_RESULT_TEXT_BYTES {
        let mut cut = MAX_RESULT_TEXT_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n… [résultat tronqué]");
        truncated = true;
        tracing::warn!(tool = tool, "résultat MCP tronqué avant affichage");
    }

    ToolCallResult {
        content,
        text,
        structured: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
        truncated,
    }
}

/// Session MCP ouverte vers un serveur, quel que soit le transport réseau.
pub enum McpSession {
    Streamable(StreamableSession),
    Sse(SseSession),
}

impl std::fmt::Debug for McpSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpSession")
            .field("transport", &self.transport())
            .finish_non_exhaustive()
    }
}

impl McpSession {
    /// Ouvre une session selon le transport configuré.
    pub async fn connect(config: ServerConfig) -> IhResult<Self> {
        match config.transport {
            McpTransport::StreamableHttp => {
                Ok(Self::Streamable(StreamableSession::connect(&config).await?))
            }
            McpTransport::Sse => Ok(Self::Sse(SseSession::connect(&config).await?)),
        }
    }

    pub fn transport(&self) -> McpTransport {
        match self {
            Self::Streamable(_) => McpTransport::StreamableHttp,
            Self::Sse(_) => McpTransport::Sse,
        }
    }

    /// Indique si la session peut encore servir un appel.
    ///
    /// Streamable HTTP est reconstruit à la demande par `rmcp` (sessions et
    /// reconnexions gérées) ; en SSE, le flux peut être interrompu et la
    /// session doit alors être fermée et reconnectée.
    pub fn is_transport_alive(&self) -> bool {
        match self {
            Self::Streamable(_) => true,
            Self::Sse(session) => session.is_connected(),
        }
    }

    /// Découvre les outils du serveur. Les schémas invalides sont exclus.
    pub async fn list_tools(&mut self) -> IhResult<Vec<ToolInfo>> {
        match self {
            Self::Streamable(session) => session.list_tools().await,
            Self::Sse(session) => session.list_tools().await,
        }
    }

    /// Appelle un outil ; les arguments doivent être un objet JSON.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> IhResult<ToolCallResult> {
        if name.trim().is_empty() {
            return Err(IhError::Invalid("nom d'outil MCP vide".into()));
        }
        if !arguments.is_object() {
            return Err(IhError::Invalid(
                "les arguments d'un outil MCP doivent être un objet JSON".into(),
            ));
        }
        match self {
            Self::Streamable(session) => session.call_tool(name, arguments).await,
            Self::Sse(session) => session.call_tool(name, arguments).await,
        }
    }

    /// Ferme proprement la session.
    pub async fn close(self) -> IhResult<()> {
        match self {
            Self::Streamable(session) => session.close().await,
            Self::Sse(session) => session.close().await,
        }
    }
}
