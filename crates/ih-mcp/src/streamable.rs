//! Transport « Streamable HTTP » via le SDK officiel `rmcp`.
//!
//! Toute l'étude du protocole (négociation, sessions `Mcp-Session-Id`, SSE de
//! réponse, reconnexions, expiration de session) est déléguée à `rmcp` ; cette
//! couche n'ajoute que ce qui relève de la politique d'IH : URL épinglée,
//! en-têtes contrôlés, bornes de taille, délai d'exécution et conversion vers
//! les types d'IH.

use crate::config::{ServerConfig, MAX_RESPONSE_BYTES, MAX_SSE_EVENT_BYTES, MAX_TOOLS};
use crate::http::{build_custom_headers, pin_endpoint};
use crate::schema::validate_tool_schema;
use crate::session::{ToolCallResult, ToolInfo};
use ih_core::{IhError, IhResult};
use ih_security::mcp::{McpToolHints, McpToolSafety};
use rmcp::model::{CallToolRequestParams, PaginatedRequestParams};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

/// Session Streamable HTTP connectée.
pub struct StreamableSession {
    service: rmcp::service::RunningService<RoleClient, ()>,
    timeout: std::time::Duration,
}

impl std::fmt::Debug for StreamableSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamableSession")
            .finish_non_exhaustive()
    }
}

impl StreamableSession {
    /// Valide l'URL, épingle les adresses, puis exécute le handshake MCP.
    ///
    /// Le handshake (échange `initialize` / `notifications/initialized`) est
    /// fait par `rmcp` : en cas de refus du serveur, l'erreur est explicite.
    pub async fn connect(config: &ServerConfig) -> IhResult<Self> {
        config.validate()?;
        let pinned = pin_endpoint(&config.url, config.policy).await?;

        let headers = build_custom_headers(&config.headers)?;
        let custom_headers: std::collections::HashMap<http::HeaderName, http::HeaderValue> =
            headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

        let mut transport_config =
            StreamableHttpClientTransportConfig::with_uri(config.url.clone())
                .custom_headers(custom_headers)
                .max_json_response_size(MAX_RESPONSE_BYTES)
                .max_sse_event_size(MAX_SSE_EVENT_BYTES)
                .reinit_on_expired_session(true);
        transport_config.allow_stateless = true;
        if let Some(token) = &config.bearer {
            transport_config = transport_config.auth_header(token.clone());
        }

        let transport = StreamableHttpClientTransport::with_client(pinned.client, transport_config);
        let service = tokio::time::timeout(
            config.timeout.max(crate::http::CONNECT_TIMEOUT) + std::time::Duration::from_secs(5),
            async { ().serve(transport).await },
        )
        .await
        .map_err(|_| {
            IhError::Extension(format!(
                "délai dépassé pendant la connexion MCP Streamable HTTP à {}",
                config.url
            ))
        })?
        .map_err(|error| {
            IhError::Extension(format!(
                "connexion MCP Streamable HTTP refusée par {} : {error}",
                config.url
            ))
        })?;

        tracing::debug!(server = config.id, "session MCP Streamable HTTP ouverte");
        Ok(Self {
            service,
            timeout: config.timeout,
        })
    }

    /// Découvre tous les outils (pagination suivie), schémas validés.
    pub async fn list_tools(&mut self) -> IhResult<Vec<ToolInfo>> {
        let timeout = self.timeout;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor
                .clone()
                .map(|cursor| PaginatedRequestParams::default().with_cursor(Some(cursor)));
            let page = tokio::time::timeout(timeout, self.service.list_tools(params))
                .await
                .map_err(|_| {
                    IhError::Extension("délai dépassé pendant tools/list (Streamable HTTP)".into())
                })?
                .map_err(|error| IhError::Extension(format!("tools/list : {error}")))?;

            for tool in page.tools {
                if tools.len() >= MAX_TOOLS {
                    return Err(IhError::Limit(format!(
                        "trop d'outils MCP (limite {MAX_TOOLS})"
                    )));
                }
                tools.push(convert_tool(tool));
            }
            // Les schémas invalides sont écartés avec une trace explicite.
            tools.retain(|tool| {
                match validate_tool_schema(&tool.input_schema) {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::warn!(tool = %tool.name, error = %error, "outil MCP ignoré : schéma invalide");
                        false
                    }
                }
            });

            match page.next_cursor {
                Some(next) if !next.is_empty() => cursor = Some(next),
                _ => break,
            }
        }
        Ok(tools)
    }

    /// Appelle un outil et renvoie une réponse bornée.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> IhResult<ToolCallResult> {
        let timeout = self.timeout;
        let arguments = arguments.as_object().ok_or_else(|| {
            IhError::Invalid("les arguments d'un outil MCP doivent être un objet JSON".into())
        })?;
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments.clone());
        let result = tokio::time::timeout(timeout, self.service.call_tool(params))
            .await
            .map_err(|_| {
                IhError::Extension(format!(
                    "délai dépassé pendant l'appel de l'outil MCP « {name} »"
                ))
            })?
            .map_err(|error| IhError::Extension(format!("appel de « {name} » : {error}")))?;
        Ok(crate::session::convert_call_result(result, name))
    }

    /// Ferme proprement la session (DELETE de session compris, géré par rmcp).
    pub async fn close(self) -> IhResult<()> {
        let service = self.service;
        service
            .cancel()
            .await
            .map_err(|error| IhError::Extension(format!("fermeture de session MCP : {error}")))?;
        Ok(())
    }
}

fn convert_tool(tool: rmcp::model::Tool) -> ToolInfo {
    let hints = tool
        .annotations
        .as_ref()
        .map(|annotations| McpToolHints {
            read_only: annotations.read_only_hint,
            destructive: annotations.destructive_hint,
            open_world: annotations.open_world_hint,
        })
        .unwrap_or_default();
    let description = tool.description.as_deref().unwrap_or_default().to_string();
    let input_schema = Value::Object((*tool.input_schema).clone());
    let safety = McpToolSafety::classify(&tool.name, &description, hints);
    ToolInfo {
        name: tool.name.to_string(),
        description,
        input_schema,
        output_schema: tool
            .output_schema
            .as_ref()
            .map(|schema| Value::Object((**schema).clone())),
        safety,
    }
}
