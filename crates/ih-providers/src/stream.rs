//! Découpage incrémental des flux : SSE (OpenAI, Anthropic, Google) et
//! NDJSON (Ollama), avec conservation des fragments partiels entre deux lectures.

use crate::common::{merge_usage, network_error};
use crate::{ChatResponse, ToolCall, Usage};
use futures_util::StreamExt;
use ih_core::{IhError, IhResult};
use reqwest::Response;
use serde_json::Value;
use std::collections::BTreeMap;

/// Accumulateur de ligne : les chunks réseau peuvent couper n'importe où,
/// y compris au milieu d'un caractère UTF-8.
#[derive(Debug, Default)]
pub(crate) struct LineDecoder {
    buffer: Vec<u8>,
}

impl LineDecoder {
    /// Ajoute un chunk et renvoie les lignes complètes qu'il termine.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        let mut consumed = 0;
        while let Some(position) = self.buffer[consumed..]
            .iter()
            .position(|byte| *byte == b'\n')
        {
            let end = consumed + position;
            lines.push(String::from_utf8_lossy(&self.buffer[consumed..end]).into_owned());
            consumed = end + 1;
        }
        if consumed > 0 {
            self.buffer.drain(..consumed);
        }
        lines
    }

    /// Renvoie la dernière ligne si le flux s'achève sans saut de ligne final.
    pub(crate) fn finish(&mut self) -> Vec<String> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        let line = String::from_utf8_lossy(&self.buffer).into_owned();
        self.buffer.clear();
        vec![line]
    }
}

/// Décodeur SSE : concatène les champs `data:` d'un événement et le publie à la
/// ligne vide. Les champs `event:`, `id:`, `retry:` et les commentaires sont
/// ignorés (le type d'événement Anthropic est présent dans le JSON).
#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    lines: LineDecoder,
    data: Vec<String>,
}

impl SseDecoder {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut events = Vec::new();
        for line in self.lines.push(chunk) {
            self.consume(&line, &mut events);
        }
        events
    }

    pub(crate) fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        for line in self.lines.finish() {
            self.consume(&line, &mut events);
        }
        self.dispatch(&mut events);
        events
    }

    fn consume(&mut self, line: &str, events: &mut Vec<String>) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            self.dispatch(events);
        } else if let Some(rest) = line.strip_prefix("data:") {
            let value = rest.strip_prefix(' ').unwrap_or(rest);
            self.data.push(value.to_string());
        }
    }

    fn dispatch(&mut self, events: &mut Vec<String>) {
        if !self.data.is_empty() {
            events.push(self.data.join("\n"));
            self.data.clear();
        }
    }
}

/// Consomme un flux SSE et s'arrête dès que `on_data` renvoie `true`.
pub(crate) async fn stream_sse<F>(
    response: Response,
    provider_id: &str,
    timeout_secs: u64,
    mut on_data: F,
) -> IhResult<()>
where
    F: FnMut(&str) -> IhResult<bool> + Send,
{
    let mut stream = response.bytes_stream();
    let mut decoder = SseDecoder::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| network_error(provider_id, timeout_secs, &error))?;
        for data in decoder.push(&chunk) {
            if data.trim().is_empty() {
                continue;
            }
            if data.trim() == "[DONE]" {
                return Ok(());
            }
            if on_data(&data)? {
                return Ok(());
            }
        }
    }
    for data in decoder.finish() {
        if data.trim().is_empty() {
            continue;
        }
        if data.trim() == "[DONE]" {
            return Ok(());
        }
        if on_data(&data)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Consomme un flux NDJSON et s'arrête dès que `on_line` renvoie `true`.
pub(crate) async fn stream_lines<F>(
    response: Response,
    provider_id: &str,
    timeout_secs: u64,
    mut on_line: F,
) -> IhResult<()>
where
    F: FnMut(&str) -> IhResult<bool> + Send,
{
    let mut stream = response.bytes_stream();
    let mut decoder = LineDecoder::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| network_error(provider_id, timeout_secs, &error))?;
        for line in decoder.push(&chunk) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if on_line(line)? {
                return Ok(());
            }
        }
    }
    for line in decoder.finish() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if on_line(line)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Accumulation d'un appel d'outil reçu fragment par fragment.
#[derive(Debug, Default)]
struct ToolCallBuffer {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallBuffer {
    fn into_tool_call(self, provider_id: &str) -> IhResult<ToolCall> {
        if self.id.is_empty() || self.name.is_empty() {
            return Err(IhError::Provider(format!(
                "appel d'outil incomplet pour {provider_id} (identifiant ou nom manquant)"
            )));
        }
        let arguments = if self.arguments.trim().is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&self.arguments).map_err(|error| {
                IhError::Provider(format!(
                    "arguments d'outil invalides pour {provider_id} ({}) : {error}",
                    self.name
                ))
            })?
        };
        Ok(ToolCall {
            id: self.id,
            name: self.name,
            arguments,
        })
    }
}

/// État agrégé d'une réponse en streaming.
#[derive(Debug)]
pub(crate) struct StreamState {
    text: String,
    model: String,
    finish_reason: Option<String>,
    usage: Usage,
    tool_calls: BTreeMap<usize, ToolCallBuffer>,
}

impl StreamState {
    pub(crate) fn new(model: &str) -> Self {
        Self {
            text: String::new(),
            model: model.to_string(),
            finish_reason: None,
            usage: Usage::default(),
            tool_calls: BTreeMap::new(),
        }
    }

    /// Le modèle annoncé par le flux prime sur celui de la requête.
    pub(crate) fn observe_model(&mut self, model: Option<&str>) {
        if let Some(model) = model.filter(|model| !model.is_empty()) {
            self.model = model.to_string();
        }
    }

    pub(crate) fn set_finish_reason(&mut self, reason: Option<&str>) {
        if let Some(reason) = reason.filter(|reason| !reason.is_empty()) {
            self.finish_reason = Some(reason.to_string());
        }
    }

    pub(crate) fn set_usage(&mut self, input: Option<u32>, output: Option<u32>) {
        merge_usage(&mut self.usage, input, output);
    }

    /// Accumule un fragment ; l'appelant décide quand le transmettre.
    pub(crate) fn append_text(&mut self, fragment: &str) {
        if !fragment.is_empty() {
            self.text.push_str(fragment);
        }
    }

    /// Accumule un fragment d'appel d'outil (identifiant du fournisseur,
    /// nom éventuellement découpé, arguments en JSON fragmenté).
    pub(crate) fn append_tool_call(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) {
        let buffer = self.tool_calls.entry(index).or_default();
        if let Some(id) = id.filter(|id| !id.is_empty()) {
            buffer.id = id.to_string();
        }
        if let Some(name) = name {
            buffer.name.push_str(name);
        }
        if let Some(arguments) = arguments {
            buffer.arguments.push_str(arguments);
        }
    }

    /// Construit la réponse finale (les arguments d'outil sont validés JSON).
    pub(crate) fn into_response(self, provider_id: &str) -> IhResult<ChatResponse> {
        let tool_calls = self
            .tool_calls
            .into_values()
            .map(|buffer| buffer.into_tool_call(provider_id))
            .collect::<IhResult<Vec<_>>>()?;
        Ok(ChatResponse {
            text: self.text,
            usage: self.usage,
            finish_reason: self.finish_reason,
            model: self.model,
            tool_calls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_reassemble_un_json_coupe_entre_deux_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"data: {\"content\":\"Bon").is_empty());
        assert_eq!(
            decoder.push(b"jour\"}\n\n"),
            vec!["{\"content\":\"Bonjour\"}"]
        );
    }

    #[test]
    fn sse_gere_plusieurs_lignes_data_et_crlf() {
        let mut decoder = SseDecoder::default();
        let events = decoder.push(b"event: ping\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\n");
        assert_eq!(events, vec!["{\"a\":\n1}"]);
    }

    #[test]
    fn sse_ne_coupe_pas_un_caractere_utf8() {
        let payload = "data: é\n\n".as_bytes();
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(&payload[..7]).is_empty());
        assert_eq!(decoder.push(&payload[7..]), vec!["é"]);
    }

    #[test]
    fn stream_state_accumule_les_appels_do_outils() {
        let mut state = StreamState::new("m");
        state.append_tool_call(0, Some("call_1"), Some("echo"), Some("{\"text\":\"a"));
        state.append_tool_call(0, None, None, Some("\"}"));
        state.append_tool_call(1, Some("call_2"), Some("add"), Some("{}"));
        let response = state.into_response("test").expect("appels valides");
        assert_eq!(response.tool_calls.len(), 2);
        assert_eq!(response.tool_calls[0].id, "call_1");
        assert_eq!(response.tool_calls[0].arguments["text"], "a");
        assert_eq!(response.tool_calls[1].name, "add");
    }

    #[test]
    fn stream_state_refuse_des_arguments_json_invalides() {
        let mut state = StreamState::new("m");
        state.append_tool_call(0, Some("call_1"), Some("echo"), Some("{pas du json"));
        let error = state.into_response("test").expect_err("JSON invalide");
        assert!(format!("{error}").contains("arguments"));
    }

    #[test]
    fn stream_state_refuse_un_appel_sans_nom() {
        let mut state = StreamState::new("m");
        state.append_tool_call(0, None, None, Some("{}"));
        assert!(state.into_response("test").is_err());
    }

    #[test]
    fn ligne_finale_sans_saut_de_ligne_est_rendue() {
        let mut decoder = LineDecoder::default();
        assert!(decoder.push(b"{\"done\":true}").is_empty());
        assert_eq!(decoder.finish(), vec!["{\"done\":true}"]);
        assert!(decoder.finish().is_empty());
    }
}
