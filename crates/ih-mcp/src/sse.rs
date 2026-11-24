//! Transport MCP SSE historique (spécification 2024-11-05).
//!
//! Le SDK officiel `rmcp` a retiré ce transport (« remove SSE transport
//! support ») : il est donc implémenté ici, mais **uniquement** la couche de
//! transport, pas le protocole JSON-RPC.
//!
//! Mécanique : `GET` sur l'URL du serveur avec `Accept: text/event-stream` ;
//! le serveur envoie d'abord un événement `endpoint` (URL de POST, relative ou
//! absolue), puis chaque requête JSON-RPC est un `POST` à cette URL et chaque
//! réponse arrive comme événement `message` du flux SSE.
//!
//! Bornes et protections :
//! - taille maximale par événement SSE et par message JSON ;
//! - délai par requête (envoi et attente), connexion initiale bornée ;
//! - reconnexions répétées avec repli exponentiel borné ; les requêtes en vol
//!   échouent explicitement quand le flux tombe ;
//! - après reconnexion, un nouveau handshake `initialize` est envoyé sur le
//!   nouveau flux (nouvelle session SSE), sans bloquer le lecteur.

use crate::config::{ServerConfig, MAX_SSE_EVENT_BYTES, MAX_TOOLS};
use crate::http::{build_custom_headers, pin_endpoint};
use crate::schema::validate_tool_schema;
use crate::session::{ToolCallResult, ToolInfo, MAX_RESULT_TEXT_BYTES};
use futures_util::StreamExt;
use ih_core::{IhError, IhResult};
use ih_security::mcp::{McpToolHints, McpToolSafety};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex, Notify};
use url::Url;

/// Nombre maximal de tentatives de reconnexion consécutives avant abandon.
const MAX_RECONNECT_ATTEMPTS: u32 = 5;
const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(300);

/// Demandes en attente : identifiant JSON-RPC → canal de réponse.
type PendingMap = HashMap<u64, oneshot::Sender<Result<Value, String>>>;
type SharedPending = Arc<Mutex<PendingMap>>;

/// Contexte partagé entre la session et sa tâche de lecture SSE.
#[derive(Clone)]
struct ReaderCtx {
    base: Url,
    client: mcp_reqwest::Client,
    stream_headers: mcp_reqwest::header::HeaderMap,
    post_headers: mcp_reqwest::header::HeaderMap,
    endpoint: Arc<Mutex<Option<Url>>>,
    endpoint_ready: Arc<Notify>,
    pending: SharedPending,
    connected: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    cancelled: Arc<AtomicBool>,
}

/// Session SSE historique connectée.
pub struct SseSession {
    config: ServerConfig,
    client: mcp_reqwest::Client,
    post_headers: mcp_reqwest::header::HeaderMap,
    endpoint: Arc<Mutex<Option<Url>>>,
    pending: SharedPending,
    next_id: AtomicU64,
    connected: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    cancelled: Arc<AtomicBool>,
    reader: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for SseSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SseSession")
            .field("url", &self.config.url)
            .field("connected", &self.connected.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl SseSession {
    /// Ouvre le flux SSE, attend l'événement `endpoint` puis exécute le
    /// handshake `initialize` / `notifications/initialized`.
    pub async fn connect(config: &ServerConfig) -> IhResult<Self> {
        config.validate()?;
        let pinned = pin_endpoint(&config.url, config.policy).await?;

        let mut stream_headers = build_custom_headers(&config.headers)?;
        stream_headers.insert(
            mcp_reqwest::header::ACCEPT,
            mcp_reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
        stream_headers.insert(
            mcp_reqwest::header::CACHE_CONTROL,
            mcp_reqwest::header::HeaderValue::from_static("no-cache"),
        );
        let mut post_headers = build_custom_headers(&config.headers)?;
        post_headers.insert(
            mcp_reqwest::header::CONTENT_TYPE,
            mcp_reqwest::header::HeaderValue::from_static("application/json"),
        );
        if let Some(token) = &config.bearer {
            let mut value = mcp_reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| {
                    IhError::Invalid("jeton bearer MCP invalide (caractères interdits)".into())
                })?;
            value.set_sensitive(true);
            stream_headers.insert(mcp_reqwest::header::AUTHORIZATION, value.clone());
            post_headers.insert(mcp_reqwest::header::AUTHORIZATION, value);
        }

        let endpoint = Arc::new(Mutex::new(None));
        let endpoint_ready = Arc::new(Notify::new());
        let pending: SharedPending = Arc::new(Mutex::new(HashMap::new()));
        let connected = Arc::new(AtomicBool::new(false));
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));

        let ctx = ReaderCtx {
            base: pinned.target.url.clone(),
            client: pinned.client.clone(),
            stream_headers,
            post_headers: post_headers.clone(),
            endpoint: endpoint.clone(),
            endpoint_ready: endpoint_ready.clone(),
            pending: pending.clone(),
            connected: connected.clone(),
            last_error: last_error.clone(),
            cancelled: cancelled.clone(),
        };
        let reader = spawn_reader(ctx);

        let mut session = Self {
            config: config.clone(),
            client: pinned.client,
            post_headers,
            endpoint: endpoint.clone(),
            pending: pending.clone(),
            next_id: AtomicU64::new(0),
            connected: connected.clone(),
            last_error: last_error.clone(),
            cancelled: cancelled.clone(),
            reader: Some(reader),
        };

        wait_for_endpoint(&session.endpoint, &endpoint_ready, config.timeout).await?;
        session.initialize().await?;
        Ok(session)
    }

    /// Dernière erreur de transport connue (diagnostic, sans secret).
    pub async fn last_transport_error(&self) -> Option<String> {
        self.last_error.lock().await.clone()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn initialize(&mut self) -> IhResult<()> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "IH",
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        let _info = self.request("initialize", Some(params)).await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    /// Découvre les outils ; les schémas invalides sont écartés.
    pub async fn list_tools(&mut self) -> IhResult<Vec<ToolInfo>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor.clone().map(|cursor| json!({ "cursor": cursor }));
            let result = self.request("tools/list", params).await?;
            let items = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    IhError::Extension(
                        "réponse tools/list MCP sans tableau « tools » (transport SSE)".into(),
                    )
                })?;
            for item in items {
                if tools.len() >= MAX_TOOLS {
                    return Err(IhError::Limit(format!(
                        "trop d'outils MCP (limite {MAX_TOOLS})"
                    )));
                }
                match convert_tool(item) {
                    Ok(tool) => {
                        if let Err(error) = validate_tool_schema(&tool.input_schema) {
                            tracing::warn!(tool = %tool.name, error = %error, "outil MCP ignoré : schéma invalide");
                            continue;
                        }
                        tools.push(tool);
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "outil MCP ignoré : entrée invalide");
                    }
                }
            }
            match result.get("nextCursor").and_then(Value::as_str) {
                Some(next) if !next.is_empty() => cursor = Some(next.to_string()),
                _ => break,
            }
        }
        Ok(tools)
    }

    /// Appelle un outil ; réponse bornée et contenu converti.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> IhResult<ToolCallResult> {
        let result = self
            .request(
                "tools/call",
                Some(json!({ "name": name, "arguments": arguments })),
            )
            .await?;
        Ok(convert_result(&result, name))
    }

    /// Ferme proprement la session (abandon du lecteur SSE).
    pub async fn close(mut self) -> IhResult<()> {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        Ok(())
    }

    async fn request(&self, method: &str, params: Option<Value>) -> IhResult<Value> {
        if !self.is_connected() {
            return Err(IhError::Extension(format!(
                "session SSE MCP non connectée (transport interrompu) : {method}"
            )));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params.unwrap_or_else(|| Value::Object(Map::new())),
        });

        let post_outcome = tokio::time::timeout(self.config.timeout, self.post(&message)).await;
        match post_outcome {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.pending.lock().await.remove(&id);
                return Err(error);
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(IhError::Extension(format!(
                    "délai de {} ms dépassé en envoyant {method} (SSE)",
                    self.config.timeout.as_millis()
                )));
            }
        }

        match tokio::time::timeout(self.config.timeout, receiver).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(IhError::Extension(message)),
            Ok(Err(_)) => Err(IhError::Extension(format!(
                "session SSE MCP fermée pendant l'attente de {method}"
            ))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(IhError::Extension(format!(
                    "délai de {} ms dépassé en attendant la réponse SSE à {method}",
                    self.config.timeout.as_millis()
                )))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> IhResult<()> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.post(&message).await
    }

    async fn post(&self, message: &Value) -> IhResult<()> {
        let endpoint =
            self.endpoint.lock().await.clone().ok_or_else(|| {
                IhError::Extension("session SSE MCP sans endpoint de POST".into())
            })?;
        let response = self
            .client
            .post(endpoint)
            .headers(self.post_headers.clone())
            .json(message)
            .send()
            .await
            .map_err(|error| IhError::Extension(format!("envoi SSE MCP impossible : {error}")))?;
        let status = response.status();
        if !status.is_success() && status != mcp_reqwest::StatusCode::ACCEPTED {
            return Err(IhError::Extension(format!(
                "envoi SSE MCP refusé : HTTP {status}"
            )));
        }
        Ok(())
    }
}

impl Drop for SseSession {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// Lecture du flux SSE
// ---------------------------------------------------------------------------

async fn wait_for_endpoint(
    endpoint: &Arc<Mutex<Option<Url>>>,
    ready: &Arc<Notify>,
    timeout: Duration,
) -> IhResult<()> {
    let notified = ready.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    if endpoint.lock().await.is_some() {
        return Ok(());
    }
    match tokio::time::timeout(timeout, notified).await {
        Ok(()) => Ok(()),
        Err(_) => Err(IhError::Extension(format!(
            "délai de {} ms dépassé en attendant l'événement « endpoint » du flux SSE MCP",
            timeout.as_millis()
        ))),
    }
}

fn spawn_reader(ctx: ReaderCtx) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let needs_reinit = Arc::new(AtomicBool::new(false));
        let mut attempts: u32 = 0;
        loop {
            let outcome = pump_stream(&ctx, &needs_reinit).await;
            if ctx.cancelled.load(Ordering::SeqCst) {
                return;
            }
            ctx.connected.store(false, Ordering::SeqCst);
            fail_pending(&ctx.pending, "session SSE MCP interrompue").await;

            match &outcome {
                Ok(true) => {
                    attempts = 0;
                    needs_reinit.store(true, Ordering::SeqCst);
                }
                Ok(false) => {
                    needs_reinit.store(true, Ordering::SeqCst);
                }
                Err(reason) => {
                    tracing::warn!(reason, "flux SSE MCP interrompu");
                    *ctx.last_error.lock().await = Some(reason.clone());
                    needs_reinit.store(true, Ordering::SeqCst);
                }
            }

            if attempts >= MAX_RECONNECT_ATTEMPTS {
                *ctx.last_error.lock().await = Some("reconnexions SSE MCP épuisées".to_string());
                tracing::warn!(attempts, "abandon de la reconnexion SSE MCP");
                return;
            }
            let delay = RECONNECT_BASE_DELAY * 2u32.saturating_pow(attempts.min(4));
            attempts += 1;
            tokio::time::sleep(delay).await;
            if ctx.cancelled.load(Ordering::SeqCst) {
                return;
            }
        }
    })
}

async fn fail_pending(pending: &SharedPending, reason: &str) {
    let mut map = pending.lock().await;
    for (_, sender) in map.drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
}

/// Pompe un flux SSE jusqu'à EOF ou erreur. Renvoie `Ok(true)` si un
/// événement `endpoint` a été reçu (session établie), `Ok(false)` sinon.
async fn pump_stream(ctx: &ReaderCtx, needs_reinit: &Arc<AtomicBool>) -> Result<bool, String> {
    let response = ctx
        .client
        .get(ctx.base.clone())
        .headers(ctx.stream_headers.clone())
        .send()
        .await
        .map_err(|error| format!("GET SSE MCP impossible : {error}"))?;
    if !response.status().is_success() {
        return Err(format!("GET SSE MCP refusé : HTTP {}", response.status()));
    }
    let content_type = response
        .headers()
        .get(mcp_reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if !content_type.starts_with("text/event-stream") {
        return Err(format!("flux SSE MCP de type inattendu : {content_type}"));
    }

    let mut stream = response.bytes_stream();
    let mut parser = SseParser::new(MAX_SSE_EVENT_BYTES);
    let mut handshake_seen = false;

    loop {
        match stream.next().await {
            Some(Ok(bytes)) => {
                let events = parser
                    .feed(&bytes)
                    .map_err(|reason| format!("flux SSE MCP invalide : {reason}"))?;
                for event in events {
                    match event.name.as_str() {
                        "endpoint" => {
                            let target = ctx
                                .base
                                .join(event.data.trim())
                                .map_err(|error| format!("endpoint SSE MCP invalide : {error}"))?;
                            *ctx.endpoint.lock().await = Some(target.clone());
                            ctx.connected.store(true, Ordering::SeqCst);
                            ctx.endpoint_ready.notify_waiters();
                            if !handshake_seen {
                                handshake_seen = true;
                            }
                            if needs_reinit.swap(false, Ordering::SeqCst) {
                                // Nouveau flux = nouvelle session côté serveur :
                                // rejouer le handshake sans bloquer le lecteur.
                                reinit_fire_and_forget(ctx, &target);
                            }
                        }
                        _ => {
                            dispatch_message(&event.data, &ctx.pending).await;
                        }
                    }
                }
            }
            Some(Err(error)) => {
                return Err(format!("interruption du flux SSE MCP : {error}"));
            }
            None => {
                return Ok(handshake_seen);
            }
        }
    }
}

fn reinit_fire_and_forget(ctx: &ReaderCtx, endpoint: &Url) {
    let client = ctx.client.clone();
    let post_headers = ctx.post_headers.clone();
    let endpoint = endpoint.clone();
    tokio::spawn(async move {
        let init = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "IH", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        if let Ok(response) = client
            .post(endpoint.clone())
            .headers(post_headers.clone())
            .json(&init)
            .send()
            .await
        {
            if response.status().is_success()
                || response.status() == mcp_reqwest::StatusCode::ACCEPTED
            {
                let _ = client
                    .post(endpoint)
                    .headers(post_headers)
                    .json(&json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/initialized",
                        "params": {},
                    }))
                    .send()
                    .await;
            }
        }
    });
}

async fn dispatch_message(data: &str, pending: &SharedPending) {
    let message: Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = %error, "message SSE MCP non JSON ignoré");
            return;
        }
    };
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        tracing::trace!("notification SSE MCP ignorée");
        return;
    };
    let Some(sender) = pending.lock().await.remove(&id) else {
        tracing::trace!(id, "réponse SSE MCP sans demandeur");
        return;
    };
    if let Some(error) = message.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .map_or("?".into(), |c| c.to_string());
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("message absent");
        let _ = sender.send(Err(format!(
            "erreur JSON-RPC SSE MCP (code {code}) : {text}"
        )));
        return;
    }
    match message.get("result") {
        Some(result) => {
            let _ = sender.send(Ok(result.clone()));
        }
        None => {
            let _ = sender.send(Err("réponse JSON-RPC SSE MCP sans « result »".into()));
        }
    }
}

// ---------------------------------------------------------------------------
// Parseur SSE
// ---------------------------------------------------------------------------

struct SseEvent {
    name: String,
    data: String,
}

struct SseParser {
    buffer: Vec<u8>,
    event_name: Option<String>,
    data: Vec<u8>,
    max_event: usize,
}

impl SseParser {
    fn new(max_event: usize) -> Self {
        Self {
            buffer: Vec::new(),
            event_name: None,
            data: Vec::new(),
            max_event,
        }
    }

    /// Ajoute des octets et renvoie les événements complets.
    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, String> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        loop {
            let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') else {
                if self.buffer.len() > self.max_event {
                    return Err("événement SSE trop volumineux".into());
                }
                break;
            };
            let mut line: Vec<u8> = self.buffer.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).map_err(|_| "ligne SSE non UTF-8".to_string())?;

            if line.is_empty() {
                if !self.data.is_empty() {
                    let mut data = String::from_utf8_lossy(&self.data).into_owned();
                    if data.ends_with('\n') {
                        data.pop();
                    }
                    events.push(SseEvent {
                        name: self.event_name.take().unwrap_or_else(|| "message".into()),
                        data,
                    });
                }
                self.data.clear();
                self.event_name = None;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line.as_str(), ""),
            };
            match field {
                "event" => self.event_name = Some(value.to_string()),
                "data" => {
                    self.data.extend_from_slice(value.as_bytes());
                    self.data.push(b'\n');
                    if self.data.len() > self.max_event {
                        return Err("événement SSE trop volumineux".into());
                    }
                }
                // `id` et `retry` n'ont pas d'incidence : la reconnexion est
                // contrôlée par IH, pas par le serveur.
                _ => {}
            }
        }
        Ok(events)
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn convert_tool(item: &Value) -> IhResult<ToolInfo> {
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| IhError::Invalid("outil MCP sans nom".into()))?
        .to_string();
    let description = item
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input_schema = item
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| json!({"type": "object"}));
    let output_schema = item.get("outputSchema").cloned();
    let annotations = item.get("annotations");
    let hints = McpToolHints {
        read_only: annotations
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(Value::as_bool),
        destructive: annotations
            .and_then(|a| a.get("destructiveHint"))
            .and_then(Value::as_bool),
        open_world: annotations
            .and_then(|a| a.get("openWorldHint"))
            .and_then(Value::as_bool),
    };
    let safety = McpToolSafety::classify(&name, &description, hints);
    Ok(ToolInfo {
        name,
        description,
        input_schema,
        output_schema,
        safety,
    })
}

fn convert_result(result: &Value, tool: &str) -> ToolCallResult {
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut text = String::new();
    for block in &content {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(chunk) = block.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(chunk);
            }
        }
    }
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
        structured: result.get("structuredContent").cloned(),
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseur_events_multi_lignes() {
        let mut parser = SseParser::new(1024);
        let events = parser.feed(b"event: message\ndata: {\"a\":1}\n\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "message");
        assert_eq!(events[0].data, "{\"a\":1}");
    }

    #[test]
    fn parseur_accepte_crlf_et_commentaires() {
        let mut parser = SseParser::new(1024);
        let events = parser
            .feed(b": commentaire\r\nevent: endpoint\r\ndata: /messages?session=1\r\n\r\n")
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "endpoint");
        assert_eq!(events[0].data, "/messages?session=1");
    }

    #[test]
    fn parseur_data_multi_lignes_jointe() {
        let mut parser = SseParser::new(1024);
        let events = parser.feed(b"data: ligne1\ndata: ligne2\n\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "message");
        assert_eq!(events[0].data, "ligne1\nligne2");
    }

    #[test]
    fn parseur_fragmenté_sur_plusieurs_chunks() {
        let mut parser = SseParser::new(1024);
        assert!(parser.feed(b"event: mess").unwrap().is_empty());
        assert!(parser.feed(b"age\ndata: {\"x\":").unwrap().is_empty());
        let events = parser.feed(b" 2}\n\n").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"x\": 2}");
    }

    #[test]
    fn parseur_refuse_evenement_trop_volumineux() {
        let mut parser = SseParser::new(64);
        let mut big = b"data: ".to_vec();
        big.extend(std::iter::repeat_n(b'x', 100));
        assert!(parser.feed(&big).is_err());
    }

    #[test]
    fn conversion_outil_risque_depuis_annotations() {
        let tool = convert_tool(&serde_json::json!({
            "name": "delete_all",
            "description": "",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true, "destructiveHint": true}
        }))
        .unwrap();
        assert_eq!(
            tool.safety.category,
            ih_security::mcp::McpToolCategory::Destructive
        );
        assert_eq!(tool.safety.risk, ih_security::risk::RiskLevel::Critical);
    }

    #[test]
    fn conversion_resultat_texte_joint() {
        let result = serde_json::json!({
            "content": [
                {"type": "text", "text": "a"},
                {"type": "text", "text": "b"},
                {"type": "image", "data": "…", "mimeType": "image/png"}
            ],
            "isError": true
        });
        let converted = convert_result(&result, "t");
        assert_eq!(converted.text, "a\nb");
        assert!(converted.is_error);
        assert_eq!(converted.content.len(), 3);
        assert!(!converted.truncated);
    }
}
