//! Fournisseur Ollama local (`POST /api/chat` en NDJSON, sans clé).

use crate::common::{
    apply_headers, build_client, check_stream_payload, custom_endpoint_url, endpoint,
    ensure_success, header_pairs, health_from_models, network_error, parse_provider_json,
    parse_stream_json, send, validate_base_url, validate_config,
};
use crate::stream::{stream_lines, StreamState};
use crate::{ChatRequest, ChatResponse, HealthReport, ModelEntry, Provider, Usage};
use async_trait::async_trait;
use ih_core::config::{ProviderConfig, ProviderKind};
use ih_core::{IhError, IhResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

/// Client d'Ollama, volontairement sans clé d'API.
#[derive(Debug)]
pub struct OllamaProvider {
    id: String,
    base_url: String,
    client: reqwest::Client,
    timeout_secs: u64,
    headers: Vec<(String, String)>,
    stream_enabled: bool,
    custom_chat_url: Option<String>,
}

impl OllamaProvider {
    pub fn new(config: &ProviderConfig) -> IhResult<Self> {
        validate_config(config, ProviderKind::Ollama)?;
        validate_base_url(config)?;
        let timeout_secs = config.timeout_secs;
        Ok(Self {
            id: config.id.clone(),
            base_url: config.base_url.trim().trim_end_matches('/').to_string(),
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

    fn chat_url(&self) -> String {
        self.custom_chat_url
            .clone()
            .unwrap_or_else(|| endpoint(&self.base_url, "api/chat"))
    }

    fn tags_url(&self) -> String {
        endpoint(&self.base_url, "api/tags")
    }

    async fn post_chat(&self, request: &ChatRequest, stream: bool) -> IhResult<reqwest::Response> {
        let builder = apply_headers(
            self.client
                .post(self.chat_url())
                .json(&request_body(request, stream)),
            &self.headers,
        );
        let response = send(builder, &self.id, self.timeout_secs).await?;
        ensure_success(response, &self.id).await
    }
}

fn request_body(request: &ChatRequest, stream: bool) -> Value {
    let mut messages = Vec::with_capacity(request.messages.len() + 1);
    if let Some(system) = request
        .system
        .as_deref()
        .filter(|system| !system.is_empty())
    {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.extend(request.messages.iter().map(|message| {
        json!({
            "role": message.role.as_str(),
            "content": message.content.as_str(),
        })
    }));
    let mut options = serde_json::Map::new();
    if let Some(temperature) = request.temperature {
        options.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_tokens) = request.max_tokens {
        options.insert("num_predict".into(), json!(max_tokens));
    }
    let mut body = json!({
        "model": request.model.as_str(),
        "messages": messages,
        "stream": stream,
    });
    if !options.is_empty() {
        body["options"] = Value::Object(options);
    }
    body
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    message: Option<OllamaMessage>,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    message: Option<OllamaMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaTags {
    #[serde(default)]
    models: Option<Vec<OllamaTag>>,
}

#[derive(Debug, Deserialize)]
struct OllamaTag {
    name: String,
    #[serde(default)]
    details: Option<OllamaDetails>,
}

#[derive(Debug, Deserialize)]
struct OllamaDetails {
    #[serde(default)]
    parameter_size: Option<String>,
}

#[async_trait]
impl Provider for OllamaProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    async fn list_models(&self) -> IhResult<Vec<ModelEntry>> {
        let builder = apply_headers(self.client.get(self.tags_url()), &self.headers);
        let response = send(builder, &self.id, self.timeout_secs).await?;
        let response = ensure_success(response, &self.id).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let payload: OllamaTags = parse_provider_json(&self.id, &body)?;
        Ok(payload
            .models
            .unwrap_or_default()
            .into_iter()
            .map(|model| {
                let id = model.name;
                let parameter_size = model
                    .details
                    .and_then(|details| details.parameter_size)
                    .filter(|size| !size.is_empty());
                let label = match parameter_size {
                    Some(size) => format!("{id} ({size})"),
                    None => id.clone(),
                };
                ModelEntry {
                    id,
                    label,
                    context_window: None,
                    supports_tools: false,
                    supports_vision: false,
                    local: true,
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
        let response = self.post_chat(&request, false).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let payload: OllamaChatResponse = parse_provider_json(&self.id, &body)?;
        let message = payload
            .message
            .ok_or_else(|| IhError::Provider(format!("réponse {} sans message", self.id)))?;
        let model = payload
            .model
            .filter(|model| !model.is_empty())
            .unwrap_or(request.model);
        Ok(ChatResponse {
            text: message.content,
            usage: Usage {
                input_tokens: payload.prompt_eval_count,
                output_tokens: payload.eval_count,
            },
            finish_reason: payload.done_reason,
            model,
            tool_calls: Vec::new(),
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
        let response = self.post_chat(&request, true).await?;
        let mut state = StreamState::new(&request.model);
        stream_lines(response, &self.id, self.timeout_secs, |line| {
            let value: Value = parse_stream_json(&self.id, line)?;
            check_stream_payload(&self.id, &value)?;
            let chunk: OllamaStreamChunk = serde_json::from_value(value).map_err(|error| {
                IhError::Provider(format!(
                    "fragment de flux invalide pour {} : {error}",
                    self.id
                ))
            })?;
            state.observe_model(chunk.model.as_deref());
            state.set_usage(chunk.prompt_eval_count, chunk.eval_count);
            if let Some(message) = chunk.message {
                if !message.content.is_empty() {
                    state.append_text(&message.content);
                    on_delta(&message.content);
                }
            }
            if chunk.done {
                state.set_finish_reason(chunk.done_reason.as_deref());
                return Ok(true);
            }
            Ok(false)
        })
        .await?;
        state.into_response(&self.id)
    }
}
