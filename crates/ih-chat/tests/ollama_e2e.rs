//! Test de bout en bout du chat avec le **vrai** client Ollama.
//!
//! Un serveur NDJSON local remplace Ollama : aucune requête ne sort de la
//! machine, mais tout le reste du chemin est réel — sérialisation de la requête,
//! parsing du flux, callback de streaming, persistance SQLite.

use ih_chat::{cancel_channel, run_turn, ChatTurn};
use ih_core::config::{ModelEntry, ProviderConfig, ProviderKind};
use ih_core::ProjectId;
use ih_providers::OllamaProvider;
use ih_store::Store;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn chunk(stream: &mut TcpStream, data: &str) {
    let _ = write!(stream, "{:x}\r\n{}\r\n", data.len(), data);
}

/// Serveur minimal qui répond une fois à `POST /api/chat` en NDJSON chunked.
fn start_fake_ollama() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("port local");
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        // Lire la requête (en-têtes + corps) sans l'interpréter.
        let mut buffer = [0u8; 16 * 1024];
        let _ = stream.read(&mut buffer);

        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\n\r\n";
        let _ = stream.write_all(head.as_bytes());
        for (content, done) in [("Bonjour", false), (" depuis ", false), ("Ollama", true)] {
            let payload = format!(
                "{{\"model\":\"fake:latest\",\"created_at\":\"2026-01-01T00:00:00Z\",\"message\":{{\"role\":\"assistant\",\"content\":\"{content}\"}},\"done\":{done},\"prompt_eval_count\":5,\"eval_count\":7}}\n"
            );
            chunk(&mut stream, &payload);
        }
        let _ = stream.write_all(b"0\r\n\r\n");
        let _ = stream.flush();
    });
    (port, handle)
}

fn provider(port: u16) -> OllamaProvider {
    let config = ProviderConfig {
        id: "ollama".into(),
        kind: ProviderKind::Ollama,
        label: "Ollama (test)".into(),
        base_url: format!("http://127.0.0.1:{port}"),
        enabled: true,
        timeout_secs: 5,
        models: vec![ModelEntry {
            id: "fake:latest".into(),
            local: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    OllamaProvider::new(&config).expect("fournisseur Ollama")
}

fn fixture() -> (Store, ih_core::ThreadId) {
    let store = Store::open_in_memory().expect("base mémoire");
    let project = ih_store::Project {
        id: ProjectId::new(),
        path: "/tmp/ih-chat-e2e".into(),
        name: "e2e".into(),
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
        trusted: true,
    };
    store.upsert_project(&project).unwrap();
    let thread = store
        .create_thread(&project.id, "Test e2e", Some("ollama"))
        .unwrap();
    (store, thread.id)
}

#[tokio::test]
async fn chat_streaming_de_bout_en_bout_avec_ollama() {
    let (port, server) = start_fake_ollama();
    let provider = provider(port);
    let (store, thread_id) = fixture();
    let mut deltas: Vec<String> = Vec::new();
    let mut sink = |delta: &str| deltas.push(delta.to_string());

    let outcome = run_turn(
        &provider,
        &store,
        &ChatTurn::new(thread_id.clone(), "ollama", "fake:latest", "Bonjour"),
        cancel_channel().1,
        &mut sink,
    )
    .await
    .expect("tour de conversation");

    server.join().expect("serveur factice");

    assert_eq!(
        deltas.concat(),
        "Bonjour depuis Ollama",
        "deltas: {deltas:?}"
    );
    assert_eq!(outcome.text, "Bonjour depuis Ollama");
    assert!(!outcome.cancelled);

    let messages = store.list_messages(&thread_id, 10, 0).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "Bonjour depuis Ollama");
    let metadata = messages[1].metadata.as_ref().unwrap();
    assert_eq!(metadata["provider"], "ollama");
    assert_eq!(metadata["cancelled"], false);
}
