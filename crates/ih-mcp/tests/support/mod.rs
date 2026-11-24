//! Serveur MCP de test local, réel (TCP, HTTP/1.1), pour les tests
//! d'intégration : aucun mock du transport — les tests exercent de vraies
//! connexions réseau sur 127.0.0.1.
//!
//! Deux transports sont servis simultanément :
//! - Streamable HTTP sur `/mcp` (POST JSON-RPC, réponses JSON) ;
//! - SSE historique sur `/sse` (GET flux, POST sur `/messages`).
//!
//! Les options permettent d'exercer les cas de sécurité : jeton exigé,
//! redirection, JSON invalide, réponse surdimensionnée, délai, outil au nom
//! destructif, coupure du flux SSE pour tester la reconnexion.

#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex, Notify};

/// Comportements dynamiques du serveur (modifiables pendant un test).
#[derive(Debug, Default, Clone)]
pub struct Mode {
    /// Répondre un JSON invalide à tools/list.
    pub bad_json: bool,
    /// Faire échouer tools/list avec une erreur JSON-RPC.
    pub fail_list: bool,
    /// tools/call renvoie un contenu texte de 9 Mio.
    pub oversized: bool,
    /// Rediriger chaque POST de /mcp vers un autre hôte (302).
    pub redirect: bool,
}

/// Outil exposé par le serveur de test.
#[derive(Debug, Clone)]
pub struct TestTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: Value,
}

impl TestTool {
    pub fn new(name: &str, schema: Value) -> Self {
        Self {
            name: name.to_string(),
            description: format!("Outil de test {name}"),
            input_schema: schema,
            annotations: Value::Null,
        }
    }
}

/// Options de démarrage.
#[derive(Debug, Clone)]
pub struct Options {
    pub bearer: Option<String>,
    pub tools: Vec<TestTool>,
    pub delay_ms: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            bearer: None,
            delay_ms: 0,
            tools: vec![
                TestTool {
                    name: "echo".into(),
                    description: "Répète le texte fourni".into(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"]
                    }),
                    annotations: json!({"readOnlyHint": true}),
                },
                TestTool::new(
                    "add",
                    json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "integer"}}, "required": ["a", "b"]}),
                ),
                TestTool::new("slow", json!({"type": "object"})),
                TestTool::new("fail", json!({"type": "object"})),
                TestTool::new("big", json!({"type": "object"})),
                TestTool::new(
                    "delete_everything",
                    json!({"type": "object", "properties": {"confirm": {"type": "boolean"}}}),
                ),
            ],
        }
    }
}

/// Serveur de test en cours d'exécution.
pub struct TestServer {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub mode: Arc<Mutex<Mode>>,
    pub delay_ms: Arc<AtomicU64>,
    pub bearer: Option<String>,
    /// Ferme les flux SSE ouverts (test de reconnexion).
    pub drop_streams: Arc<Notify>,
    stop: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url())
    }
    pub fn sse_url(&self) -> String {
        format!("{}/sse", self.base_url())
    }
    pub async fn shutdown(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.task.abort();
    }
}

/// Démarre le serveur.
pub async fn start(options: Options) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind 127.0.0.1:0");
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mode = Arc::new(Mutex::new(Mode::default()));
    let delay_ms = Arc::new(AtomicU64::new(options.delay_ms));
    let drop_streams = Arc::new(Notify::new());
    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(ServerState {
        mode: mode.clone(),
        payload: options.clone(),
        delay_ms: delay_ms.clone(),
        requests: requests.clone(),
        drop_streams: drop_streams.clone(),
        stop: stop.clone(),
        sessions: Mutex::new(HashMap::new()),
    });

    let server_state = state.clone();
    let task = tokio::spawn(async move {
        loop {
            let accepted = listener.accept().await;
            let Ok((stream, _)) = accepted else { break };
            let state = server_state.clone();
            tokio::spawn(async move {
                let _ = handle_connection(stream, state).await;
            });
        }
    });

    TestServer {
        port,
        requests,
        mode,
        delay_ms,
        bearer: options.bearer,
        drop_streams,
        stop,
        task,
    }
}

struct ServerState {
    mode: Arc<Mutex<Mode>>,
    payload: Options,
    delay_ms: Arc<AtomicU64>,
    requests: Arc<Mutex<Vec<String>>>,
    drop_streams: Arc<Notify>,
    stop: Arc<AtomicBool>,
    /// Flux SSE par session : canal vers la connexion GET ouverte.
    sessions: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(|v| v.as_str())
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<ServerState>) -> std::io::Result<()> {
    loop {
        let Some(request) = read_request(&mut stream).await? else {
            return Ok(());
        };
        let path_only = request
            .path
            .split('?')
            .next()
            .unwrap_or(&request.path)
            .to_string();
        let method = request.method.clone();
        state
            .requests
            .lock()
            .await
            .push(format!("{method} {path_only}"));

        if let Some(bearer) = &state.payload.bearer {
            let expected = format!("Bearer {bearer}");
            if request.header("authorization") != Some(expected.as_str()) {
                respond(
                    &mut stream,
                    401,
                    "Unauthorized",
                    Some("application/json"),
                    b"{\"error\":\"jeton manquant ou invalide\"}".to_vec(),
                )
                .await?;
                continue;
            }
        }

        match (request.method.as_str(), path_only.as_str()) {
            ("POST", "/mcp") => {
                let mode = state.mode.lock().await.clone();
                if mode.redirect {
                    let location = "http://127.0.0.1:1/evil";
                    respond_with(
                        &mut stream,
                        302,
                        "Found",
                        Some(("location", location)),
                        b"".to_vec(),
                    )
                    .await?;
                    continue;
                }
                let response = process_streamable(&request.body, &state).await;
                respond(&mut stream, 200, "OK", Some("application/json"), response).await?;
            }
            ("GET", "/mcp") => {
                // Pas de flux serveur : le transport le tolère (405).
                respond(&mut stream, 405, "Method Not Allowed", None, b"".to_vec()).await?;
            }
            ("DELETE", "/mcp") => {
                respond(&mut stream, 200, "OK", None, b"".to_vec()).await?;
            }
            ("GET", "/sse") => {
                let session = format!("s{}", unique_id());
                let (tx, mut rx) = mpsc::unbounded_channel::<String>();
                state.sessions.lock().await.insert(session.clone(), tx);
                let endpoint = format!("/messages?session={session}");
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\nevent: endpoint\ndata: {endpoint}\n\n"
                );
                stream.write_all(head.as_bytes()).await?;
                stream.flush().await?;

                let drop = state.drop_streams.notified();
                tokio::pin!(drop);
                loop {
                    tokio::select! {
                        _ = &mut drop => {
                            break;
                        }
                        message = rx.recv() => {
                            match message {
                                Some(text) => {
                                    let chunk = format!("event: message\ndata: {text}\n\n");
                                    if stream.write_all(chunk.as_bytes()).await.is_err() {
                                        break;
                                    }
                                    let _ = stream.flush().await;
                                }
                                None => break,
                            }
                        }
                    }
                }
                state.sessions.lock().await.remove(&session);
                let _ = stream.shutdown().await;
                return Ok(());
            }
            ("POST", "/messages") => {
                let session = request
                    .path
                    .split("session=")
                    .nth(1)
                    .unwrap_or_default()
                    .split('&')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let body = request.body.clone();
                let state_for_task = state.clone();
                tokio::spawn(async move {
                    let response = process_sse_message(&body, &state_for_task).await;
                    if let Some(tx) = state_for_task.sessions.lock().await.get(&session) {
                        let _ = tx.send(response);
                    }
                });
                respond(&mut stream, 202, "Accepted", None, b"".to_vec()).await?;
            }
            _ => {
                respond(&mut stream, 404, "Not Found", None, b"".to_vec()).await?;
            }
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    // Lecture jusqu'à la fin des en-têtes.
    let header_end = loop {
        if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 1024 * 1024 {
            return Ok(None);
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(Some(HttpRequest {
        method,
        path,
        headers,
        body,
    }))
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> std::io::Result<()> {
    respond_with(
        stream,
        status,
        reason,
        content_type.map(|value| ("content-type", value)),
        body,
    )
    .await
}

async fn respond_with(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    extra_header: Option<(&str, &str)>,
    body: Vec<u8>,
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some((name, value)) = extra_header {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(&body).await?;
    }
    stream.flush().await?;
    Ok(())
}

fn unique_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

async fn process_streamable(body: &[u8], state: &ServerState) -> Vec<u8> {
    let request: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => return b"{}".to_vec(),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    let response = match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "ih-test-mcp", "version": "1.0.0"}
            }
        }),
        "notifications/initialized" | "notifications/cancelled" => {
            return b"".to_vec();
        }
        "tools/list" => {
            let mode = state.mode.lock().await.clone();
            if mode.fail_list {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": "list indisponible"}})
            } else if mode.bad_json {
                return b"{\"jsonrpc\":\"2.0\",".to_vec();
            } else {
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"tools": state.payload.tools.iter().map(|tool| json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                        "annotations": tool.annotations,
                    })).collect::<Vec<_>>()}
                })
            }
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let delay = state.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            match name.as_str() {
                "echo" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": arguments.get("text").cloned().unwrap_or(json!(""))}],
                        "isError": false
                    }
                }),
                "add" => {
                    let a = arguments.get("a").and_then(Value::as_i64).unwrap_or(0);
                    let b = arguments.get("b").and_then(Value::as_i64).unwrap_or(0);
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {"content": [{"type": "text", "text": (a + b).to_string()}]}
                    })
                }
                "slow" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": "lent"}]}
                }),
                "big" => {
                    let mode = state.mode.lock().await.clone();
                    if mode.oversized {
                        let text = "x".repeat(9 * 1024 * 1024);
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"content": [{"type": "text", "text": text}]}
                        })
                    } else {
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {"content": [{"type": "text", "text": "petit"}]}
                        })
                    }
                }
                "delete_everything" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": "supprimé"}]}
                }),
                "fail" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "outil introuvable: fail"}
                }),
                _ => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32602, "message": format!("outil inconnu: {name}")}
                }),
            }
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("méthode inconnue: {method}")}
        }),
    };
    serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec())
}

async fn process_sse_message(body: &[u8], state: &ServerState) -> String {
    let request: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(_) => {
            return "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32700,\"message\":\"parse\"}}".into()
        }
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let response = match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "ih-test-sse", "version": "1.0.0"}
            }
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": state.payload.tools.iter().map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
                "annotations": tool.annotations,
            })).collect::<Vec<_>>()}
        }),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let delay = state.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            match name.as_str() {
                "echo" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": params.get("arguments").and_then(|a| a.get("text")).cloned().unwrap_or(json!(""))}]}
                }),
                "fail" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "outil introuvable: fail"}
                }),
                _ => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": "ok"}]}
                }),
            }
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("méthode inconnue: {method}")}
        }),
    };
    serde_json::to_string(&response).unwrap_or_default()
}
