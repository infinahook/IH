//! Tests de bout en bout avec un serveur HTTP factice local.
//!
//! Aucune requête ne sort de la machine : le serveur écoute sur `127.0.0.1`
//! avec un port éphémère et répond en HTTP/1.1 (corps simple ou chunked pour
//! les flux). Les assertions portent aussi sur les en-têtes et les corps reçus.

use ih_core::config::{ProviderConfig, ProviderKind};
use ih_core::IhError;
use ih_providers::{
    build_provider, AnthropicProvider, ChatMessage, ChatRequest, GoogleProvider, OllamaProvider,
    OpenAiCompatibleProvider, Provider,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

type Handler = dyn Fn(&FakeRequest) -> FakeResponse + Send + Sync;

#[derive(Clone, Debug)]
struct FakeRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: String,
}

#[derive(Clone)]
struct FakeResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    chunks: Option<Vec<Vec<u8>>>,
    delay: Duration,
}

impl FakeResponse {
    fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: body.into().into_bytes(),
            chunks: None,
            delay: Duration::ZERO,
        }
    }

    fn sse_chunks(chunks: Vec<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            body: Vec::new(),
            chunks: Some(chunks.into_iter().map(String::into_bytes).collect()),
            delay: Duration::ZERO,
        }
    }

    fn ndjson_chunks(chunks: Vec<String>) -> Self {
        Self {
            status: 200,
            content_type: "application/x-ndjson",
            body: Vec::new(),
            chunks: Some(chunks.into_iter().map(String::into_bytes).collect()),
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.into().into_bytes(),
            chunks: None,
            delay: Duration::ZERO,
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct FakeServer {
    addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<FakeRequest>>>,
    worker: Option<JoinHandle<()>>,
}

impl FakeServer {
    fn start<H>(handler: H) -> Self
    where
        H: Fn(&FakeRequest) -> FakeResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind du serveur factice");
        listener
            .set_nonblocking(true)
            .expect("passage en non bloquant");
        let addr = listener.local_addr().expect("adresse locale");
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let worker_shutdown = shutdown.clone();
        let worker_requests = requests.clone();
        let worker = thread::spawn(move || loop {
            if worker_shutdown.load(Ordering::Relaxed) {
                break;
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    serve(stream, &*handler, &worker_requests, &worker_shutdown);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });
        Self {
            addr,
            shutdown,
            requests,
            worker: Some(worker),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<FakeRequest> {
        self.requests.lock().expect("verrou des requêtes").clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve(
    mut stream: TcpStream,
    handler: &Handler,
    requests: &Mutex<Vec<FakeRequest>>,
    shutdown: &AtomicBool,
) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    requests
        .lock()
        .expect("verrou des requêtes")
        .push(request.clone());
    let response = handler(&request);
    if !sleep_interruptible(response.delay, shutdown) {
        return;
    }
    let _ = write_response(&mut stream, &response);
}

fn read_request(stream: &mut TcpStream) -> Option<FakeRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buffer.len() > 128 * 1024 {
            return None;
        }
    };
    let headers_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = headers_text.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = String::from_utf8_lossy(
        buffer
            .get(header_end..header_end + content_length)
            .unwrap_or_default(),
    )
    .into_owned();
    Some(FakeRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_response(stream: &mut TcpStream, response: &FakeResponse) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nConnection: close\r\n",
        response.status,
        reason(response.status),
        response.content_type
    );
    if let Some(chunks) = &response.chunks {
        head.push_str("Transfer-Encoding: chunked\r\n\r\n");
        stream.write_all(head.as_bytes())?;
        for chunk in chunks {
            stream.write_all(format!("{:x}\r\n", chunk.len()).as_bytes())?;
            stream.write_all(chunk)?;
            stream.write_all(b"\r\n")?;
            stream.flush()?;
            thread::sleep(Duration::from_millis(2));
        }
        stream.write_all(b"0\r\n\r\n")?;
    } else {
        head.push_str(&format!("Content-Length: {}\r\n\r\n", response.body.len()));
        stream.write_all(head.as_bytes())?;
        stream.write_all(&response.body)?;
    }
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn sleep_interruptible(duration: Duration, shutdown: &AtomicBool) -> bool {
    let step = Duration::from_millis(10);
    let mut remaining = duration;
    while remaining > Duration::ZERO {
        if shutdown.load(Ordering::Relaxed) {
            return false;
        }
        let slice = remaining.min(step);
        thread::sleep(slice);
        remaining -= slice;
    }
    !shutdown.load(Ordering::Relaxed)
}

fn path_of(request: &FakeRequest) -> &str {
    request.path.split('?').next().unwrap_or(&request.path)
}

fn provider_config(id: &str, kind: ProviderKind, base_url: String) -> ProviderConfig {
    ProviderConfig {
        id: id.into(),
        kind,
        label: id.into(),
        base_url,
        enabled: true,
        default_model: None,
        models: Vec::new(),
        has_secret: true,
        timeout_secs: 10,
    }
}

fn chat_request(model: &str, prompt: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: prompt.into(),
        }],
        system: Some("Tu es IH.".into()),
        temperature: Some(0.5),
        max_tokens: Some(64),
    }
}

fn openai_provider(server: &FakeServer) -> Box<dyn Provider> {
    let config = provider_config(
        "openai",
        ProviderKind::OpenaiCompatible,
        format!("{}/v1", server.base_url()),
    );
    build_provider(&config, Some("sk-test".into())).expect("fournisseur OpenAI")
}

#[tokio::test]
async fn openai_chat_non_streaming_texte_et_usage() {
    let server = FakeServer::start(|request| {
        if request.method == "POST" && path_of(request) == "/v1/chat/completions" {
            FakeResponse::json(
                r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"gpt-4o-mini","choices":[{"index":0,"message":{"role":"assistant","content":"Bonjour !"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3,"total_tokens":15}}"#,
            )
        } else {
            FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#)
        }
    });
    let provider = openai_provider(&server);
    let response = provider
        .chat(chat_request("gpt-4o-mini", "Bonjour"))
        .await
        .expect("réponse de chat");

    assert_eq!(response.text, "Bonjour !");
    assert_eq!(response.usage.input_tokens, Some(12));
    assert_eq!(response.usage.output_tokens, Some(3));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.model, "gpt-4o-mini");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    let body: Value = serde_json::from_str(&requests[0].body).expect("corps JSON");
    assert_eq!(body["stream"].as_bool(), Some(false));
    assert_eq!(body["messages"][0]["role"].as_str(), Some("system"));
    assert_eq!(body["messages"][0]["content"].as_str(), Some("Tu es IH."));
    assert_eq!(body["messages"][1]["content"].as_str(), Some("Bonjour"));
    assert_eq!(body["max_tokens"].as_u64(), Some(64));
    assert_eq!(body["temperature"].as_f64(), Some(0.5));
}

#[tokio::test]
async fn openai_streaming_sse_deltas_dans_l_ordre() {
    let server = FakeServer::start(|request| {
        if request.method == "POST" && path_of(request) == "/v1/chat/completions" {
            FakeResponse::sse_chunks(vec![
                "data: {\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"Bon"
                    .to_string(),
                "jour\"},\"finish_reason\":null}]}\n\n".to_string(),
                "data: {\"choices\":[{\"delta\":{\"content\":\" le \"}}]}\n\n".to_string(),
                "data: {\"choices\":[{\"delta\":{\"content\":\"monde\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n".to_string(),
                "data: [DONE]\n\n".to_string(),
            ])
        } else {
            FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#)
        }
    });
    let provider = openai_provider(&server);
    let mut deltas = Vec::new();
    let response = provider
        .chat_stream(chat_request("gpt-4o-mini", "Bonjour"), &mut |delta| {
            deltas.push(delta.to_string());
        })
        .await
        .expect("flux SSE");

    assert_eq!(deltas, vec!["Bonjour", " le ", "monde"]);
    assert_eq!(response.text, "Bonjour le monde");
    assert_eq!(response.usage.input_tokens, Some(7));
    assert_eq!(response.usage.output_tokens, Some(3));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.model, "gpt-4o-mini");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_str(&requests[0].body).expect("corps JSON");
    assert_eq!(body["stream"].as_bool(), Some(true));
}

#[tokio::test]
async fn anthropic_streaming_content_block_delta() {
    let server = FakeServer::start(|request| {
        if request.method == "POST" && path_of(request) == "/v1/messages" {
            FakeResponse::sse_chunks(vec![
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4\",\"usage\":{\"input_tokens\":9,\"output_tokens\":1}}}\n\n".to_string(),
                "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n".to_string(),
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Salut\"}}\n\n".to_string(),
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" !\"}}\n\n".to_string(),
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":4}}\n\n".to_string(),
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string(),
            ])
        } else {
            FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#)
        }
    });
    let config = provider_config("anthropic", ProviderKind::Anthropic, server.base_url());
    let provider = build_provider(&config, Some("sk-ant-test".into())).expect("fournisseur");
    let mut deltas = Vec::new();
    let response = provider
        .chat_stream(chat_request("claude-sonnet-4", "Bonjour"), &mut |delta| {
            deltas.push(delta.to_string());
        })
        .await
        .expect("flux Anthropic");

    assert_eq!(deltas, vec!["Salut", " !"]);
    assert_eq!(response.text, "Salut !");
    assert_eq!(response.usage.input_tokens, Some(9));
    assert_eq!(response.usage.output_tokens, Some(4));
    assert_eq!(response.finish_reason.as_deref(), Some("end_turn"));
    assert_eq!(response.model, "claude-sonnet-4");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].headers.get("x-api-key").map(String::as_str),
        Some("sk-ant-test")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("anthropic-version")
            .map(String::as_str),
        Some("2023-06-01")
    );
    assert!(!requests[0].headers.contains_key("authorization"));
    let body: Value = serde_json::from_str(&requests[0].body).expect("corps JSON");
    assert_eq!(body["stream"].as_bool(), Some(true));
    assert_eq!(body["max_tokens"].as_u64(), Some(64));
    assert_eq!(body["system"].as_str(), Some("Tu es IH."));
}

#[tokio::test]
async fn google_generate_content_et_list_models() {
    let server = FakeServer::start(|request| {
        let path = path_of(request);
        if request.method == "POST" && path == "/v1beta/models/gemini-2.0-flash:generateContent" {
            FakeResponse::json(
                r#"{"candidates":[{"content":{"parts":[{"text":"Bonjour"}],"role":"model"},"finishReason":"STOP","index":0}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"totalTokenCount":7},"modelVersion":"gemini-2.0-flash"}"#,
            )
        } else if request.method == "GET" && path == "/v1beta/models" {
            FakeResponse::json(
                r#"{"models":[{"name":"models/gemini-2.0-flash","displayName":"Gemini 2.0 Flash","inputTokenLimit":1048576,"outputTokenLimit":8192,"supportedGenerationMethods":["generateContent"]},{"name":"models/gemini-embedding-001","displayName":"Gemini Embedding","inputTokenLimit":2048}]}"#,
            )
        } else {
            FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#)
        }
    });
    let config = provider_config("google", ProviderKind::Google, server.base_url());
    let provider = build_provider(&config, Some("goog-test".into())).expect("fournisseur");
    let response = provider
        .chat(chat_request("gemini-2.0-flash", "Bonjour"))
        .await
        .expect("generateContent");
    assert_eq!(response.text, "Bonjour");
    assert_eq!(response.usage.input_tokens, Some(5));
    assert_eq!(response.usage.output_tokens, Some(2));
    assert_eq!(response.finish_reason.as_deref(), Some("STOP"));
    assert_eq!(response.model, "gemini-2.0-flash");

    let models = provider.list_models().await.expect("liste des modèles");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gemini-2.0-flash");
    assert_eq!(models[0].label, "Gemini 2.0 Flash");
    assert_eq!(models[0].context_window, Some(1_048_576));
    assert!(!models[0].supports_tools);
    assert!(!models[0].supports_vision);
    assert!(!models[0].local);
    assert_eq!(models[1].id, "gemini-embedding-001");
    assert_eq!(models[1].context_window, Some(2048));

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]
            .headers
            .get("x-goog-api-key")
            .map(String::as_str),
        Some("goog-test")
    );
    let body: Value = serde_json::from_str(&requests[0].body).expect("corps JSON");
    assert_eq!(
        body["contents"][0]["parts"][0]["text"].as_str(),
        Some("Bonjour")
    );
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"].as_str(),
        Some("Tu es IH.")
    );
    assert_eq!(
        body["generationConfig"]["maxOutputTokens"].as_u64(),
        Some(64)
    );
}

#[tokio::test]
async fn google_streaming_alt_sse() {
    let server = FakeServer::start(|request| {
        if request.method == "POST"
            && path_of(request) == "/v1beta/models/gemini-2.0-flash:streamGenerateContent"
            && request.path.contains("alt=sse")
        {
            FakeResponse::sse_chunks(vec![
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Bon\"}]}}],\"modelVersion\":\"gemini-2.0-flash\"}\n\n".to_string(),
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"jour\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2}}\n\n".to_string(),
            ])
        } else {
            FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#)
        }
    });
    let config = provider_config("google", ProviderKind::Google, server.base_url());
    let provider = build_provider(&config, Some("goog-test".into())).expect("fournisseur");
    let mut deltas = Vec::new();
    let response = provider
        .chat_stream(chat_request("gemini-2.0-flash", "Bonjour"), &mut |delta| {
            deltas.push(delta.to_string());
        })
        .await
        .expect("flux Google");
    assert_eq!(deltas, vec!["Bon", "jour"]);
    assert_eq!(response.text, "Bonjour");
    assert_eq!(response.usage.input_tokens, Some(3));
    assert_eq!(response.usage.output_tokens, Some(2));
    assert_eq!(response.finish_reason.as_deref(), Some("STOP"));
    assert_eq!(response.model, "gemini-2.0-flash");
}

#[tokio::test]
async fn ollama_tags_et_chat_ndjson() {
    let server = FakeServer::start(|request| {
        match (request.method.as_str(), path_of(request)) {
            ("GET", "/api/tags") => FakeResponse::json(
                r#"{"models":[{"name":"llama3.2:latest","model":"llama3.2:latest","details":{"parent_model":"","format":"gguf","family":"llama","families":["llama"],"parameter_size":"3.2B","quantization_level":"Q4_K_M"}},{"name":"qwen2.5-coder:7b","details":{"parameter_size":"7.4B"}},{"name":"mistral:latest"}]}"#,
            ),
            ("POST", "/api/chat") => FakeResponse::ndjson_chunks(vec![
                "{\"model\":\"llama3.2:latest\",\"message\":{\"role\":\"assistant\",\"content\":\"Bon\"},\"done\":false}\n".to_string(),
                "{\"model\":\"llama3.2:latest\",\"message\":{\"role\":\"assistant\",\"content\":\"jour\"},\"done\":false}\n".to_string(),
                "{\"model\":\"llama3.2:latest\",\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":11,\"eval_count\":4}".to_string(),
            ]),
            _ => FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#),
        }
    });
    let config = provider_config("ollama", ProviderKind::Ollama, server.base_url());
    let provider = build_provider(&config, None).expect("fournisseur Ollama");

    let models = provider.list_models().await.expect("liste Ollama");
    assert_eq!(models.len(), 3);
    assert_eq!(models[0].id, "llama3.2:latest");
    assert_eq!(models[0].label, "llama3.2:latest (3.2B)");
    assert!(models[0].local);
    assert_eq!(models[0].context_window, None);
    assert!(!models[0].supports_tools);
    assert_eq!(models[1].label, "qwen2.5-coder:7b (7.4B)");
    assert_eq!(models[2].label, "mistral:latest");

    let mut deltas = Vec::new();
    let response = provider
        .chat_stream(chat_request("llama3.2:latest", "Bonjour"), &mut |delta| {
            deltas.push(delta.to_string());
        })
        .await
        .expect("flux Ollama");
    assert_eq!(deltas, vec!["Bon", "jour"]);
    assert_eq!(response.text, "Bonjour");
    assert_eq!(response.usage.input_tokens, Some(11));
    assert_eq!(response.usage.output_tokens, Some(4));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(response.model, "llama3.2:latest");

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| !request.headers.contains_key("authorization")));
    let body: Value = serde_json::from_str(&requests[1].body).expect("corps JSON");
    assert_eq!(body["stream"].as_bool(), Some(true));
    assert_eq!(body["options"]["num_predict"].as_u64(), Some(64));
}

#[tokio::test]
async fn erreur_401_message_explicite() {
    let server = FakeServer::start(|_| {
        FakeResponse::status(
            401,
            r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#,
        )
    });
    let provider = openai_provider(&server);
    let error = provider
        .chat(chat_request("gpt-4o-mini", "Bonjour"))
        .await
        .expect_err("401 attendu");
    match error {
        IhError::Provider(message) => {
            let lower = message.to_lowercase();
            assert!(lower.contains("clé d'api invalide"), "{message}");
            assert!(message.contains("401"), "{message}");
            assert!(message.contains("Incorrect API key provided"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn secret_absent_echoue_avant_toute_connexion() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("non bloquant");
    let addr = listener.local_addr().expect("adresse");
    for (kind, base_url) in [
        (ProviderKind::OpenaiCompatible, format!("http://{addr}/v1")),
        (ProviderKind::Anthropic, format!("http://{addr}")),
        (ProviderKind::Google, format!("http://{addr}")),
    ] {
        let config = provider_config("cloud", kind, base_url);
        let error = build_provider(&config, None).err().expect("clé absente");
        assert!(matches!(error, IhError::Provider(_)), "{error}");
        assert!(
            error.to_string().contains("clé d'API absente pour cloud"),
            "{error}"
        );
    }
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "aucune connexion ne doit être tentée sans clé"
    );
    drop(listener);

    // Variante demandée : le port est maintenant fermé. Si une requête avait
    // été tentée, l'erreur serait `Network` (connexion refusée), pas `Provider`.
    let config = provider_config(
        "cloud-ferme",
        ProviderKind::OpenaiCompatible,
        format!("http://{addr}/v1"),
    );
    let error = build_provider(&config, None)
        .err()
        .expect("clé absente sur port fermé");
    assert!(matches!(error, IhError::Provider(_)), "{error}");

    let config = provider_config("ollama", ProviderKind::Ollama, format!("http://{addr}"));
    assert!(build_provider(&config, None).is_ok());
}

#[tokio::test]
async fn timeout_court_renvoie_une_erreur_reseau_sans_panique() {
    let server = FakeServer::start(|_| FakeResponse::json("{}").delayed(Duration::from_secs(5)));
    let mut config = provider_config(
        "openai",
        ProviderKind::OpenaiCompatible,
        format!("{}/v1", server.base_url()),
    );
    config.timeout_secs = 1;
    let provider = build_provider(&config, Some("sk-test".into())).expect("fournisseur");
    let started = Instant::now();
    let error = provider
        .chat(chat_request("gpt-4o-mini", "Bonjour"))
        .await
        .expect_err("timeout attendu");
    let elapsed = started.elapsed();
    assert!(matches!(error, IhError::Network(_)), "{error}");
    assert!(error.to_string().contains("délai dépassé"), "{error}");
    assert!(
        elapsed < Duration::from_secs(4),
        "timeout trop long : {elapsed:?}"
    );
}

#[tokio::test]
async fn openai_list_models_parse_une_charge_realiste() {
    let server = FakeServer::start(|request| {
        if request.method == "GET" && path_of(request) == "/v1/models" {
            FakeResponse::json(
                r#"{"object":"list","data":[{"id":"gpt-4o-mini","object":"model","created":1721172740,"owned_by":"system"},{"id":"openrouter/auto","name":"Auto Router","context_length":2000000,"supported_parameters":["tools","temperature"],"architecture":{"input_modalities":["text","image"]},"top_provider":{"context_length":2000000}}]}"#,
            )
        } else {
            FakeResponse::status(404, r#"{"error":{"message":"route inconnue"}}"#)
        }
    });
    let provider = openai_provider(&server);
    let models = provider.list_models().await.expect("liste OpenAI");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "gpt-4o-mini");
    assert_eq!(models[0].label, "gpt-4o-mini");
    assert_eq!(models[0].context_window, None);
    assert!(!models[0].supports_tools);
    assert!(!models[0].supports_vision);
    assert_eq!(models[1].id, "openrouter/auto");
    assert_eq!(models[1].label, "Auto Router");
    assert_eq!(models[1].context_window, Some(2_000_000));
    assert!(models[1].supports_tools);
    assert!(models[1].supports_vision);
    assert!(models.iter().all(|model| !model.local));

    let report = provider.health().await.expect("santé");
    assert!(report.ok);
    assert_eq!(report.detail, "2 modèle(s) disponible(s)");
    assert!(report.latency_ms < 10_000);
}

#[test]
fn constructeurs_refusent_un_kind_incompatible() {
    let config = provider_config(
        "google",
        ProviderKind::Google,
        "https://generativelanguage.googleapis.com".into(),
    );
    assert!(matches!(
        AnthropicProvider::new(&config, Some("secret".into())),
        Err(IhError::Config(_))
    ));
    assert!(matches!(
        OpenAiCompatibleProvider::new(&config, Some("secret".into())),
        Err(IhError::Config(_))
    ));
    assert!(matches!(
        OllamaProvider::new(&config),
        Err(IhError::Config(_))
    ));

    let config = provider_config("ollama", ProviderKind::Ollama, "http://127.0.0.1:1".into());
    assert!(matches!(
        GoogleProvider::new(&config, Some("secret".into())),
        Err(IhError::Config(_))
    ));
}

/// Contrat de compilation : la signature publique accepte bien un objet
/// `&mut dyn FnMut(&str)` (cas d'usage d'un callback stocké par l'appelant).
#[allow(dead_code)]
async fn signature_accepte_un_callback_dyn(
    provider: &dyn Provider,
    request: ChatRequest,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> ih_core::IhResult<ih_providers::ChatResponse> {
    provider.chat_stream(request, on_delta).await
}
