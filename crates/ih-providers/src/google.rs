//! Fournisseur Google Gemini (`generateContent` et `streamGenerateContent`).

use crate::common::{
    apply_headers, build_client, check_stream_payload, custom_endpoint_url, ensure_success,
    header_pairs, health_from_models, network_error, parse_provider_json, parse_stream_json,
    require_secret, send, validate_base_url, validate_config, versioned_endpoint,
};
use crate::stream::{stream_sse, StreamState};
use crate::{ChatRequest, ChatResponse, HealthReport, ModelEntry, Provider};
use async_trait::async_trait;
use ih_core::config::{ProviderConfig, ProviderKind};
use ih_core::{IhError, IhResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;

/// Client de l'API Generative Language de Google.
pub struct GoogleProvider {
    id: String,
    base_url: String,
    secret: String,
    client: reqwest::Client,
    timeout_secs: u64,
    headers: Vec<(String, String)>,
    stream_enabled: bool,
    custom_chat_url: Option<String>,
}

impl std::fmt::Debug for GoogleProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GoogleProvider")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl GoogleProvider {
    pub fn new(config: &ProviderConfig, secret: Option<String>) -> IhResult<Self> {
        validate_config(config, ProviderKind::Google)?;
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

    fn generate_url(&self, model: &str, stream: bool) -> String {
        let model = model.strip_prefix("models/").unwrap_or(model);
        let encoded = model
            .split('/')
            .map(|segment| urlencoding::encode(segment).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let action = if stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        if let Some(custom) = &self.custom_chat_url {
            // Endpoint personnalisé : l'appelant fournit le chemin complet.
            return if stream {
                format!(
                    "{custom}{}alt=sse",
                    if custom.contains('?') { "&" } else { "?" }
                )
            } else {
                custom.clone()
            };
        }
        versioned_endpoint(
            &self.base_url,
            "v1beta",
            &format!("models/{encoded}:{action}"),
        )
    }

    fn models_url(&self) -> String {
        versioned_endpoint(&self.base_url, "v1beta", "models")
    }

    fn auth(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header("x-goog-api-key", &self.secret)
    }

    async fn post_generate(
        &self,
        request: &ChatRequest,
        stream: bool,
    ) -> IhResult<reqwest::Response> {
        let builder = apply_headers(
            self.auth(self.client.post(self.generate_url(&request.model, stream)))
                .json(&request_body(request)),
            &self.headers,
        );
        let response = send(builder, &self.id, self.timeout_secs).await?;
        ensure_success(response, &self.id).await
    }
}

fn request_body(request: &ChatRequest) -> Value {
    let mut system_parts: Vec<&str> = Vec::new();
    if let Some(system) = request
        .system
        .as_deref()
        .filter(|system| !system.is_empty())
    {
        system_parts.push(system);
    }
    let mut contents = Vec::with_capacity(request.messages.len());
    for message in &request.messages {
        if message.role.eq_ignore_ascii_case("system") {
            if !message.content.is_empty() {
                system_parts.push(&message.content);
            }
            continue;
        }
        let role = if message.role.eq_ignore_ascii_case("assistant")
            || message.role.eq_ignore_ascii_case("model")
        {
            "model"
        } else {
            "user"
        };
        contents.push(json!({
            "role": role,
            "parts": [{"text": message.content.as_str()}],
        }));
    }
    let mut body = json!({"contents": contents});
    if !system_parts.is_empty() {
        body["systemInstruction"] = json!({"parts": [{"text": system_parts.join("\n\n")}]});
    }
    let mut generation = serde_json::Map::new();
    if let Some(temperature) = request.temperature {
        generation.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_tokens) = request.max_tokens {
        generation.insert("maxOutputTokens".into(), json!(max_tokens));
    }
    if !generation.is_empty() {
        body["generationConfig"] = Value::Object(generation);
    }
    body
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleResponse {
    #[serde(default)]
    candidates: Vec<GoogleCandidate>,
    #[serde(default)]
    usage_metadata: Option<GoogleUsage>,
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    prompt_feedback: Option<PromptFeedback>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCandidate {
    #[serde(default)]
    content: Option<GoogleContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleContent {
    #[serde(default)]
    parts: Vec<GooglePart>,
}

#[derive(Debug, Deserialize)]
struct GooglePart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleUsage {
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(default)]
    candidates_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    #[serde(default)]
    block_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleModelsResponse {
    #[serde(default)]
    models: Option<Vec<GoogleModel>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleModel {
    name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    input_token_limit: Option<u64>,
}

/// Fusionne un événement Google dans l'état courant ; renvoie `(candidat vu,
/// raison de blocage éventuelle)`.
fn absorb(
    response: GoogleResponse,
    state: &mut StreamState,
    fragments: &mut Vec<String>,
) -> (bool, Option<String>) {
    let GoogleResponse {
        candidates,
        usage_metadata,
        model_version,
        prompt_feedback,
    } = response;
    let mut saw_candidate = false;
    for candidate in candidates {
        saw_candidate = true;
        if let Some(content) = candidate.content {
            for part in content.parts {
                if let Some(text) = part.text {
                    if !text.is_empty() {
                        state.append_text(&text);
                        fragments.push(text);
                    }
                }
            }
        }
        state.set_finish_reason(candidate.finish_reason.as_deref());
    }
    if let Some(usage) = usage_metadata {
        state.set_usage(usage.prompt_token_count, usage.candidates_token_count);
    }
    state.observe_model(model_version.as_deref());
    let block_reason = prompt_feedback.and_then(|feedback| feedback.block_reason);
    (saw_candidate, block_reason)
}

fn blocked(provider_id: &str, reason: &str) -> IhError {
    IhError::Provider(format!(
        "contenu bloqué par le fournisseur {provider_id} : {reason}"
    ))
}

#[async_trait]
impl Provider for GoogleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Google
    }

    async fn list_models(&self) -> IhResult<Vec<ModelEntry>> {
        let builder = apply_headers(self.auth(self.client.get(self.models_url())), &self.headers);
        let response = send(builder, &self.id, self.timeout_secs).await?;
        let response = ensure_success(response, &self.id).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let payload: GoogleModelsResponse = parse_provider_json(&self.id, &body)?;
        Ok(payload
            .models
            .unwrap_or_default()
            .into_iter()
            .map(|model| {
                let id = model
                    .name
                    .strip_prefix("models/")
                    .unwrap_or(&model.name)
                    .to_string();
                let label = model
                    .display_name
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| id.clone());
                ModelEntry {
                    id,
                    label,
                    context_window: model
                        .input_token_limit
                        .and_then(|tokens| u32::try_from(tokens).ok()),
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
        let model = request
            .model
            .strip_prefix("models/")
            .unwrap_or(&request.model)
            .to_string();
        let response = self.post_generate(&request, false).await?;
        let body = response
            .text()
            .await
            .map_err(|error| network_error(&self.id, self.timeout_secs, &error))?;
        let payload: GoogleResponse = parse_provider_json(&self.id, &body)?;
        let mut state = StreamState::new(&model);
        let mut fragments = Vec::new();
        let (saw_candidate, block_reason) = absorb(payload, &mut state, &mut fragments);
        if let Some(reason) = block_reason {
            return Err(blocked(&self.id, &reason));
        }
        if !saw_candidate {
            return Err(IhError::Provider(format!(
                "réponse {} sans candidat",
                self.id
            )));
        }
        state.into_response(&self.id)
    }

    async fn chat_stream(
        &self,
        request: ChatRequest,
        on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> IhResult<ChatResponse> {
        let model = request
            .model
            .strip_prefix("models/")
            .unwrap_or(&request.model)
            .to_string();
        if !self.stream_enabled {
            let response = self.chat(request).await?;
            on_delta(&response.text);
            return Ok(response);
        }
        let response = self.post_generate(&request, true).await?;
        let mut state = StreamState::new(&model);
        stream_sse(response, &self.id, self.timeout_secs, |data| {
            let value: Value = parse_stream_json(&self.id, data)?;
            check_stream_payload(&self.id, &value)?;
            let event: GoogleResponse = serde_json::from_value(value).map_err(|error| {
                IhError::Provider(format!(
                    "fragment de flux invalide pour {} : {error}",
                    self.id
                ))
            })?;
            let mut fragments = Vec::new();
            let (_, block_reason) = absorb(event, &mut state, &mut fragments);
            for fragment in &fragments {
                on_delta(fragment);
            }
            if let Some(reason) = block_reason {
                return Err(blocked(&self.id, &reason));
            }
            Ok(false)
        })
        .await?;
        state.into_response(&self.id)
    }
}
