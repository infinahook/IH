//! Fournisseur compatible OpenAI (`POST /chat/completions`, SSE `data:`).

use crate::common::{
    apply_headers, build_client, check_stream_payload, custom_endpoint_url, endpoint,
    ensure_success, header_pairs, health_from_models, network_error, parse_provider_json,
    parse_stream_json, require_secret, send, validate_base_url, validate_config,
};
use crate::stream::{stream_sse, StreamState};
use crate::{
    ChatMessage, ChatRequest, ChatResponse, HealthReport, ModelEntry, Provider, ToolCall,
    ToolDefinition, Usage,
};
use async_trait::async_trait;
use ih_core::config::{ProviderConfig, ProviderKind};
use ih_core::{IhError, IhResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

/// Client des API compatibles OpenAI (OpenAI, OpenRouter, LM Studio, vLLM…).
pub struct OpenAiCompatibleProvider {
    id: String,
    base_url: String,
    secret: String,
    client: reqwest::Client,
    timeout_secs: u64,
    headers: Vec<(String, String)>,
    stream_enabled: bool,
    custom_chat_url: Option<String>,
    supports_tools: bool,
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleProvider")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl OpenAiCompatibleProvider {
    pub fn new(config: &ProviderConfig, secret: Option<String>) -> IhResult<Self> {
        validate_config(config, ProviderKind::OpenaiCompatible)?;
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
            supports_tools: config.supports_tools,
        })
    }

    fn chat_url(&self) -> String {
        self.custom_chat_url
            .clone()
            .unwrap_or_else(|| endpoint(&self.base_url, "chat/completions"))
    }

    fn models_url(&self) -> String {
        endpoint(&self.base_url, "models")
    }

    async fn post_chat(&self, request: &ChatRequest, stream: bool) -> IhResult<reqwest::Response> {
        let builder = apply_headers(
            self.client
                .post(self.chat_url())
                .bearer_auth(&self.secret)
                .json(&request_body(request, stream)),
            &self.headers,
        );
        let response = send(builder, &self.id, self.timeout_secs).await?;
        ensure_success(response, &self.id).await
    }
}

/// Message au format OpenAI (assistant avec `tool_calls`, message `tool`…).
fn message_body(message: &ChatMessage) -> Value {
    let mut body = json!({"role": message.role.as_str()});
    if message.content.is_empty() && !message.tool_calls.is_empty() {
        // Un assistant qui demande des outils n'a pas de contenu texte.
        body["content"] = Value::Null;
    } else {
        body["content"] = json!(message.content.as_str());
    }
    if let Some(tool_call_id) = &message.tool_call_id {
        body["tool_call_id"] = json!(tool_call_id);
    }
    if !message.tool_calls.is_empty() {
        let calls = message
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "id": call.id.as_str(),
                    "type": "function",
                    "function": {
                        "name": call.name.as_str(),
                        "arguments": call.arguments.to_string(),
                    },
                })
            })
            .collect::<Vec<_>>();
        body["tool_calls"] = json!(calls);
    }
    body
}

/// Outils proposés au modèle au format OpenAI.
fn tools_payload(tools: &[ToolDefinition]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name.as_str(),
                        "description": tool.description.as_str(),
                        "parameters": tool.parameters.clone(),
                    },
                })
            })
            .collect(),
    )
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
    messages.extend(request.messages.iter().map(message_body));
    let mut body = json!({
        "model": request.model.as_str(),
        "messages": messages,
        "stream": stream,
    });
    if !request.tools.is_empty() {
        body["tools"] = tools_payload(&request.tools);
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(max_tokens) = request.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    body
}

/// Convertit les arguments d'un appel d'outil tels que reçus par le fournisseur
/// (chaîne JSON, objet, ou absent) en valeur JSON validée.
fn parse_tool_arguments(
    provider_id: &str,
    tool_name: &str,
    raw: Option<&Value>,
) -> IhResult<Value> {
    match raw {
        None | Some(Value::Null) => Ok(Value::Object(serde_json::Map::new())),
        Some(Value::String(text)) => {
            if text.trim().is_empty() {
                Ok(Value::Object(serde_json::Map::new()))
            } else {
                serde_json::from_str(text).map_err(|error| {
                    IhError::Provider(format!(
                        "arguments d'outil invalides pour {provider_id} ({tool_name}) : {error}"
                    ))
                })
            }
        }
        Some(value) => Ok(value.clone()),
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletion {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatChoiceMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    tool_calls: Option<Vec<RawToolCall>>,
}

#[derive(Debug, Deserialize)]
struct RawToolCall {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<RawFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct RawFunctionCall {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<StreamDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    /// OpenRouter (et d'autres passerelles) exposent cette information.
    #[serde(default)]
    context_length: Option<u64>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_context_length: Option<u64>,
    #[serde(default)]
    supported_parameters: Option<Vec<String>>,
    #[serde(default)]
    architecture: Option<ModelArchitecture>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
}

#[derive(Debug, Deserialize)]
struct ModelArchitecture {
    #[serde(default)]
    input_modalities: Option<Vec<String>>,
    #[serde(default)]
    modality: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<u64>,
}

impl From<OpenAiModel> for ModelEntry {
    fn from(model: OpenAiModel) -> Self {
        let OpenAiModel {
            id,
            name,
            context_length,
            context_window,
            max_context_length,
            supported_parameters,
            architecture,
            top_provider,
        } = model;
        let context_window = context_length
            .or(context_window)
            .or(max_context_length)
            .or_else(|| top_provider.and_then(|provider| provider.context_length))
            .and_then(|tokens| u32::try_from(tokens).ok());
        let supports_tools = supported_parameters
            .unwrap_or_default()
            .iter()
            .any(|parameter| parameter == "tools");
        let supports_vision = architecture.is_some_and(|architecture| {
            architecture
                .input_modalities
                .is_some_and(|modalities| modalities.iter().any(|modality| modality == "image"))
                || architecture
                    .modality
                    .is_some_and(|modality| modality.contains("image"))
        });
        let label = name
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.clone());
        Self {
            id,
            label,
            context_window,
            supports_tools,
            supports_vision,
            local: false,
        }
    }
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        Value::Null | Value::Number(_) | Value::Bool(_) | Value::Object(_) => String::new(),
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenaiCompatible
    }

    fn supports_native_tools(&self) -> bool {
        self.supports_tools
    }

    async fn list_models(&self) -> IhResult<Vec<ModelEntry>> {
        let builder = apply_headers(
            self.client.get(self.models_url()).bearer_auth(&self.secret),
            &self.headers,
        );
        let response = send(builder, &self.id, self.timeout_secs).await?;
        let response = ensure_success(response, &self.id).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let models: ModelsResponse = parse_provider_json(&self.id, &body)?;
        Ok(models.data.into_iter().map(ModelEntry::from).collect())
    }

    async fn health(&self) -> IhResult<HealthReport> {
        let started = Instant::now();
        let models = self.list_models().await;
        Ok(health_from_models(started, models))
    }

    async fn chat(&self, request: ChatRequest) -> IhResult<ChatResponse> {
        if !request.tools.is_empty() && !self.supports_tools {
            return Err(IhError::Provider(format!(
                "tool calling natif non activé pour {}",
                self.id
            )));
        }
        let response = self.post_chat(&request, false).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let completion: ChatCompletion = parse_provider_json(&self.id, &body)?;
        let choice = completion
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| IhError::Provider(format!("réponse {} sans choix", self.id)))?;
        let ChatChoice {
            message,
            finish_reason,
        } = choice;
        let (text, tool_calls) = match message {
            Some(message) => {
                let text = message
                    .content
                    .map(|content| content_text(&content))
                    .unwrap_or_default();
                let raw_calls = message.tool_calls.unwrap_or_default();
                let mut tool_calls = Vec::with_capacity(raw_calls.len());
                for raw in raw_calls {
                    let function = raw.function.ok_or_else(|| {
                        IhError::Provider(format!("appel d'outil sans fonction pour {}", self.id))
                    })?;
                    let name = function.name.unwrap_or_default();
                    if name.is_empty() {
                        return Err(IhError::Provider(format!(
                            "appel d'outil sans nom pour {}",
                            self.id
                        )));
                    }
                    let id = raw
                        .id
                        .filter(|id| !id.is_empty())
                        .unwrap_or_else(|| format!("call_{}", tool_calls.len()));
                    let arguments =
                        parse_tool_arguments(&self.id, &name, function.arguments.as_ref())?;
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                (text, tool_calls)
            }
            None => (String::new(), Vec::new()),
        };
        let usage = completion.usage.unwrap_or_default();
        let model = completion
            .model
            .filter(|model| !model.is_empty())
            .unwrap_or(request.model);
        Ok(ChatResponse {
            text,
            usage: Usage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
            },
            finish_reason,
            model,
            tool_calls,
        })
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> IhResult<ChatResponse> {
        if !request.tools.is_empty() && !self.supports_tools {
            return Err(IhError::Provider(format!(
                "tool calling natif non activé pour {}",
                self.id
            )));
        }
        if !self.stream_enabled {
            let response = self.chat(request).await?;
            on_delta(&response.text);
            return Ok(response);
        }
        let response = self.post_chat(&request, true).await?;
        let mut state = StreamState::new(&request.model);
        stream_sse(response, &self.id, self.timeout_secs, |data| {
            let value: Value = parse_stream_json(&self.id, data)?;
            check_stream_payload(&self.id, &value)?;
            let chunk: StreamChunk = serde_json::from_value(value).map_err(|error| {
                IhError::Provider(format!(
                    "fragment de flux invalide pour {} : {error}",
                    self.id
                ))
            })?;
            state.observe_model(chunk.model.as_deref());
            if let Some(usage) = &chunk.usage {
                state.set_usage(usage.prompt_tokens, usage.completion_tokens);
            }
            for choice in chunk.choices {
                state.set_finish_reason(choice.finish_reason.as_deref());
                if let Some(delta) = choice.delta {
                    if let Some(content) = delta.content {
                        if !content.is_empty() {
                            state.append_text(&content);
                            on_delta(&content);
                        }
                    }
                    for (position, call) in
                        delta.tool_calls.unwrap_or_default().into_iter().enumerate()
                    {
                        let index = call.index.unwrap_or(position);
                        let (name, arguments) = call
                            .function
                            .map(|function| (function.name, function.arguments))
                            .unwrap_or((None, None));
                        state.append_tool_call(
                            index,
                            call.id.as_deref(),
                            name.as_deref(),
                            arguments.as_deref(),
                        );
                    }
                }
            }
            Ok(false)
        })
        .await?;
        state.into_response(&self.id)
    }
}
