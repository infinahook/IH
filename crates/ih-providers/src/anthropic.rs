//! Fournisseur Anthropic (`POST /v1/messages`, SSE `content_block_delta`).

use crate::common::{
    apply_headers, build_client, check_stream_payload, custom_endpoint_url, ensure_success,
    header_pairs, health_from_models, network_error, parse_provider_json, parse_stream_json,
    require_secret, send, validate_base_url, validate_config, versioned_endpoint,
    DEFAULT_MAX_TOKENS,
};
use crate::stream::{stream_sse, StreamState};
use crate::{ChatRequest, ChatResponse, HealthReport, ModelEntry, Provider, Usage};
use async_trait::async_trait;
use ih_core::config::{ProviderConfig, ProviderKind};
use ih_core::{IhError, IhResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Client de l'API Messages d'Anthropic.
pub struct AnthropicProvider {
    id: String,
    base_url: String,
    secret: String,
    client: reqwest::Client,
    timeout_secs: u64,
    headers: Vec<(String, String)>,
    stream_enabled: bool,
    custom_chat_url: Option<String>,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AnthropicProvider")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl AnthropicProvider {
    pub fn new(config: &ProviderConfig, secret: Option<String>) -> IhResult<Self> {
        validate_config(config, ProviderKind::Anthropic)?;
        let secret = require_secret(config, secret)?;
        validate_base_url(config)?;
        let timeout_secs = config.timeout_secs;
        Ok(Self {
            id: config.id.clone(),
            base_url: config.base_url.trim().trim_end_matches('/').to_string(),
            secret,
            client: build_client(timeout_secs)?,
            timeout_secs,
            headers: header_pairs(config),
            stream_enabled: config.streaming,
            custom_chat_url: config
                .endpoint_path
                .as_deref()
                .map(|path| custom_endpoint_url(&config.base_url, path)),
        })
    }

    fn messages_url(&self) -> String {
        self.custom_chat_url
            .clone()
            .unwrap_or_else(|| versioned_endpoint(&self.base_url, "v1", "messages"))
    }

    fn models_url(&self) -> String {
        versioned_endpoint(&self.base_url, "v1", "models")
    }

    fn auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("x-api-key", &self.secret)
            .header("anthropic-version", ANTHROPIC_VERSION)
    }

    async fn post_messages(
        &self,
        request: &ChatRequest,
        stream: bool,
    ) -> IhResult<reqwest::Response> {
        let builder = apply_headers(
            self.auth(self.client.post(self.messages_url()))
                .json(&request_body(request, stream)),
            &self.headers,
        );
        let response = send(builder, &self.id, self.timeout_secs).await?;
        ensure_success(response, &self.id).await
    }
}

fn request_body(request: &ChatRequest, stream: bool) -> Value {
    let messages = request
        .messages
        .iter()
        .map(|message| {
            json!({
                "role": message.role.as_str(),
                "content": message.content.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": request.model.as_str(),
        "max_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": messages,
    });
    if let Some(system) = request
        .system
        .as_deref()
        .filter(|system| !system.is_empty())
    {
        body["system"] = json!(system);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if stream {
        body["stream"] = json!(true);
    }
    body
}

#[derive(Debug, Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    content: Vec<ContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    message: Option<StreamMessage>,
    #[serde(default)]
    delta: Option<StreamDelta>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamMessage {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    #[serde(default)]
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
    /// Présent sur certains proxys ; l'API publique Anthropic ne l'expose pas.
    #[serde(default)]
    context_window: Option<u32>,
}

fn content_text(content: Vec<ContentBlock>) -> String {
    content
        .into_iter()
        .filter(|block| block.block_type == "text")
        .filter_map(|block| block.text)
        .collect()
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Anthropic
    }

    async fn list_models(&self) -> IhResult<Vec<ModelEntry>> {
        let builder = apply_headers(self.auth(self.client.get(self.models_url())), &self.headers);
        let response = send(builder, &self.id, self.timeout_secs).await?;
        let response = ensure_success(response, &self.id).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let models: AnthropicModelsResponse = parse_provider_json(&self.id, &body)?;
        Ok(models
            .data
            .into_iter()
            .map(|model| {
                let label = model
                    .display_name
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| model.id.clone());
                ModelEntry {
                    id: model.id,
                    label,
                    context_window: model.context_window,
                    supports_tools: false,
                    supports_vision: false,
                    local: false,
                }
            })
            .collect())
    }

    async fn health(&self) -> IhResult<HealthReport> {
        let started = Instant::now();
        let models = self.list_models().await;
        Ok(health_from_models(started, models))
    }

    async fn chat(&self, request: ChatRequest) -> IhResult<ChatResponse> {
        let response = self.post_messages(&request, false).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let response: MessagesResponse = parse_provider_json(&self.id, &body)?;
        let usage = response.usage.unwrap_or_default();
        let model = response
            .model
            .filter(|model| !model.is_empty())
            .unwrap_or(request.model);
        Ok(ChatResponse {
            text: content_text(response.content),
            usage: Usage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
            },
            finish_reason: response.stop_reason,
            model,
        })
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> IhResult<ChatResponse> {
        if !self.stream_enabled {
            let response = self.chat(request).await?;
            on_delta(&response.text);
            return Ok(response);
        }
        let response = self.post_messages(&request, true).await?;
        let mut state = StreamState::new(&request.model);
        stream_sse(response, &self.id, self.timeout_secs, |data| {
            let value: Value = parse_stream_json(&self.id, data)?;
            check_stream_payload(&self.id, &value)?;
            let event: StreamEvent = serde_json::from_value(value).map_err(|error| {
                IhError::Provider(format!(
                    "fragment de flux invalide pour {} : {error}",
                    self.id
                ))
            })?;
            match event.event_type.as_str() {
                "message_start" => {
                    if let Some(message) = event.message {
                        state.observe_model(message.model.as_deref());
                        if let Some(usage) = message.usage {
                            state.set_usage(usage.input_tokens, None);
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = event.delta {
                        if let Some(text) = delta.text {
                            if !text.is_empty() {
                                state.append_text(&text);
                                on_delta(&text);
                            }
                        }
                    }
                }
                "message_delta" => {
                    if let Some(delta) = event.delta {
                        state.set_finish_reason(delta.stop_reason.as_deref());
                    }
                    if let Some(usage) = event.usage {
                        state.set_usage(usage.input_tokens, usage.output_tokens);
                    }
                }
                _ => {}
            }
            Ok(false)
        })
        .await?;
        Ok(state.into_response())
    }
}
