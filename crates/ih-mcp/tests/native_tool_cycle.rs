//! Cycle complet d'outils MCP natifs, en conditions réelles locales :
//! un **vrai serveur MCP** (TCP/HTTP, `tests/support`) et un **fournisseur
//! OpenAI simulé** (serveur HTTP local) qui émet un appel d'outil puis reçoit
//! le résultat et continue.
//!
//! Ce test couvre la chaîne : définition d'outil → requête du modèle →
//! proposition persistée → approbation liée à l'empreinte → exécution MCP
//! réelle → message `tool` → continuation → réponse finale. Le seul maillon
//! non couvert ici est la couche IPC Tauri (`chat_send`/`chat_tool_decide`),
//! qui délègue exactement à ces composants et est exercée en recette
//! graphique.

mod support;

use ih_chat::{cancel_channel, run_turn, ChatTurn};
use ih_core::{ProjectId, ThreadId};
use ih_mcp::{McpSession, ServerConfig};
use ih_providers::{OpenAiCompatibleProvider, Provider, ToolCall, ToolDefinition};
use ih_security::approval::{ApprovalDecision, ApprovalRegistry};
use ih_security::mcp::mcp_call_fingerprint;
use ih_store::{NewChatToolCall, NewMessage, Store};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Fournisseur OpenAI simulé : répond un appel d'outil, puis le texte final.
// ---------------------------------------------------------------------------

struct FakeOpenAi {
    port: u16,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl FakeOpenAi {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("adresse").port();
        let requests: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let requests_for_task = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let requests = requests_for_task.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, requests).await;
                });
            }
        });
        Self { port, requests }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    async fn requests(&self) -> Vec<Value> {
        self.requests.lock().await.clone()
    }
}

async fn serve(mut stream: TcpStream, requests: Arc<Mutex<Vec<Value>>>) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut header_end = None;
    loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            if let Some(position) = find(&buffer, b"\r\n\r\n") {
                header_end = Some(position + 4);
            }
        }
        let Some(end) = header_end else { continue };
        let headers = String::from_utf8_lossy(&buffer[..end]).to_string();
        let length = headers
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if buffer.len() >= end + length {
            let body = String::from_utf8_lossy(&buffer[end..end + length]).to_string();
            let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let has_tool_result = payload["messages"]
                .as_array()
                .map(|messages| {
                    messages
                        .iter()
                        .any(|message| message["role"].as_str() == Some("tool"))
                })
                .unwrap_or(false);
            {
                let mut guard = requests.lock().await;
                guard.push(payload.clone());
            }
            let response = if has_tool_result {
                // Cycle 2 : le modèle reçoit le résultat et conclut.
                json!({
                    "model": "simule-1",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "Le serveur MCP répond : bonjour."},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 30, "completion_tokens": 8}
                })
            } else {
                // Cycle 1 : le modèle demande l'outil exposé par le catalogue.
                json!({
                    "model": "simule-1",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [{
                                "id": "call_echo_1",
                                "type": "function",
                                "function": {
                                    "name": "demo__echo",
                                    "arguments": "{\"text\":\"bonjour\"}"
                                }
                            }]
                        },
                        "finish_reason": "tool_calls"
                    }],
                    "usage": {"prompt_tokens": 25, "completion_tokens": 12}
                })
            };
            let body = response.to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(reply.as_bytes()).await?;
            stream.flush().await?;
            return Ok(());
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Fixture : base SQLite, conversation, serveur MCP réel, fournisseur simulé.
// ---------------------------------------------------------------------------

struct Fixture {
    store: Store,
    thread: ThreadId,
    project: ProjectId,
    session: McpSession,
    provider: OpenAiCompatibleProvider,
    fake: FakeOpenAi,
    definitions: Vec<ToolDefinition>,
}

async fn fixture() -> Fixture {
    let server = support::start(support::Options {
        tools: vec![
            support::TestTool::new(
                "echo",
                json!({
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }),
            ),
            support::TestTool::new(
                "add",
                json!({
                    "type": "object",
                    "properties": {"a": {"type": "number"}, "b": {"type": "number"}},
                    "required": ["a", "b"]
                }),
            ),
        ],
        ..Default::default()
    })
    .await;

    let config = ServerConfig::new(
        "demo",
        server.mcp_url(),
        ih_mcp::McpTransport::StreamableHttp,
    );
    let mut session = McpSession::connect(config).await.expect("session MCP");
    let tools = session.list_tools().await.expect("catalogue MCP");
    assert_eq!(tools.len(), 2);
    let definitions: Vec<ToolDefinition> = tools
        .iter()
        .map(|tool| ToolDefinition {
            name: format!("demo__{}", tool.name),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        })
        .collect();

    let store = Store::open_in_memory().expect("base mémoire");
    let project = ProjectId::new();
    store
        .upsert_project(&ih_store::Project {
            id: project.clone(),
            path: "/tmp/ih-native-tools".into(),
            name: "native-tools".into(),
            created_at: chrono::Utc::now(),
            last_opened_at: chrono::Utc::now(),
            trusted: true,
        })
        .expect("projet");
    let thread = store
        .create_thread(&project, "Essai natif", None)
        .expect("conversation")
        .id;

    let fake = FakeOpenAi::start().await;
    let provider_config = ih_core::config::ProviderConfig {
        id: "simule".into(),
        kind: ih_core::config::ProviderKind::OpenaiCompatible,
        label: "Simulé".into(),
        base_url: fake.base_url(),
        enabled: true,
        supports_tools: true,
        streaming: false,
        timeout_secs: 10,
        ..Default::default()
    };
    let provider = OpenAiCompatibleProvider::new(&provider_config, Some("sk-test".into()))
        .expect("fournisseur");

    Fixture {
        store,
        thread,
        project,
        session,
        provider,
        fake,
        definitions,
    }
}

// ---------------------------------------------------------------------------
// Le cycle complet.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cycle_natif_du_modèle_au_résultat_approuvé_et_à_la_continuation() {
    let mut fixture = fixture().await;
    assert!(fixture.provider.supports_native_tools());

    // 1. Tour initial : le modèle reçoit les outils et demande un appel.
    let mut turn = ChatTurn::new(
        fixture.thread.clone(),
        "simule",
        "simule-1",
        "Appelle l'écho",
    );
    turn.tools = fixture.definitions.clone();
    let (_canceller, token) = cancel_channel();
    let outcome = run_turn(
        &fixture.provider,
        &fixture.store,
        &turn,
        token,
        &mut |_delta| {},
    )
    .await
    .expect("tour initial");
    assert_eq!(outcome.tool_calls.len(), 1);
    let call: ToolCall = outcome.tool_calls[0].clone();
    assert_eq!(call.name, "demo__echo");
    let requests = fixture.fake.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["tools"][0]["function"]["name"], "demo__echo");

    // 2. Proposition persistée, en attente de décision humaine.
    let arguments = call.arguments.clone();
    let fingerprint = mcp_call_fingerprint("demo", "echo", &arguments);
    let proposition = fixture
        .store
        .insert_chat_tool_call(&NewChatToolCall {
            thread_id: fixture.thread.clone(),
            project_id: fixture.project.clone(),
            request_id: "req-1".into(),
            round: 0,
            provider_call_id: call.id.clone(),
            exposed_name: call.name.clone(),
            server_id: "demo".into(),
            tool: "echo".into(),
            risk: "low".into(),
            category: "read".into(),
            fingerprint: fingerprint.clone(),
            arguments: ih_security::mcp::redact_arguments(&arguments),
            raw_arguments: arguments.clone(),
            status: "pending".into(),
            error: None,
        })
        .expect("proposition");
    assert_eq!(proposition.status, "pending");

    // 3. Décision humaine : approbation à usage unique liée à l'empreinte.
    let approvals = ApprovalRegistry::default_ttl();
    let request = approvals
        .ask_with_fingerprint(
            "mcp_tool",
            "Écho",
            "Paramètres",
            ih_security::risk::RiskLevel::Low,
            Some(fingerprint.clone()),
        )
        .expect("demande d'approbation");
    let approval_id = request.id.into_string();
    approvals
        .resolve(&approval_id, ApprovalDecision::Approve)
        .expect("approbation");
    // Empreinte différente : refus explicite, jamais de rejeu.
    assert!(approvals
        .consume_with_fingerprint(
            &approval_id,
            "mcp_tool",
            &mcp_call_fingerprint("demo", "echo", &json!({"text": "autre"})),
        )
        .is_err());
    let ticket = approvals
        .consume_with_fingerprint(&approval_id, "mcp_tool", &fingerprint)
        .expect("consommation");
    assert_eq!(ticket.category, "mcp_tool");

    // 4. Exécution réelle sur le serveur MCP local, via la session.
    let result = fixture
        .session
        .call_tool("echo", arguments.clone())
        .await
        .expect("appel MCP réel");
    assert!(!result.is_error);
    assert!(
        result.text.contains("bonjour"),
        "résultat : {}",
        result.text
    );

    // 5. Clôture de la proposition et message `tool` renvoyé au modèle.
    let closed = fixture
        .store
        .finish_chat_tool_call(
            &proposition.id,
            "completed",
            None,
            Some(&result.text),
            Some(&approval_id),
        )
        .expect("clôture");
    assert_eq!(closed.status, "completed");
    fixture
        .store
        .append_message(&NewMessage {
            thread_id: fixture.thread.clone(),
            role: "tool".into(),
            content: result.text.clone(),
            metadata: Some(json!({
                "source": "mcp-tool",
                "tool_call_id": call.id,
                "tool": "echo",
                "server_id": "demo",
                "status": "completed",
            })),
        })
        .expect("message tool");

    // 6. Continuation : le modèle reçoit assistant.tool_calls + message tool.
    let mut continuation = ChatTurn::continuation(fixture.thread.clone(), "simule", "simule-1");
    continuation.tools = fixture.definitions.clone();
    let (_canceller, token) = cancel_channel();
    let final_outcome = run_turn(
        &fixture.provider,
        &fixture.store,
        &continuation,
        token,
        &mut |_delta| {},
    )
    .await
    .expect("continuation");
    assert_eq!(final_outcome.tool_calls.len(), 0);
    assert!(
        final_outcome.text.contains("bonjour"),
        "texte final : {}",
        final_outcome.text
    );

    let requests = fixture.fake.requests().await;
    assert_eq!(requests.len(), 2);
    let second = &requests[1]["messages"].as_array().expect("messages");
    let assistant_with_calls = second
        .iter()
        .find(|message| {
            message["role"] == "assistant" && message["tool_calls"].as_array().is_some()
        })
        .expect("assistant avec tool_calls rejoué");
    assert_eq!(assistant_with_calls["tool_calls"][0]["id"], "call_echo_1");
    let tool_message = second
        .iter()
        .find(|message| message["role"] == "tool")
        .expect("message tool transmis");
    assert_eq!(tool_message["tool_call_id"], "call_echo_1");
    assert!(tool_message["content"]
        .as_str()
        .unwrap_or_default()
        .contains("bonjour"));

    // 7. La proposition n'est plus en attente : aucune décision double.
    assert!(fixture
        .store
        .decide_chat_tool_call(&proposition.id, "denied", None)
        .is_err());
    // Le tour de continuation n'a pas ajouté de message utilisateur.
    let persisted = fixture
        .store
        .list_messages(&fixture.thread, 50, 0)
        .expect("historique");
    let users = persisted
        .iter()
        .filter(|message| message.role == "user")
        .count();
    assert_eq!(users, 1, "un seul message utilisateur pour tout le tour");

    fixture.session.close().await.expect("fermeture");
}
