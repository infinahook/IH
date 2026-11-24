//! Serveur MCP de démonstration (Streamable HTTP, réponses JSON).
//!
//! Lancement : `cargo run -p ih-mcp --example demo_server -- 18765`
//!
//! Expose `POST /mcp` avec les méthodes JSON-RPC `initialize`, `tools/list` et
//! `tools/call` ; outils : `echo` (lecture), `add` (lecture), `fail` (erreur
//! JSON-RPC) et `delete_everything` (classé destructif par IH). Aucun accès
//! réseau sortant : uniquement 127.0.0.1.
//!
//! Ce serveur existe pour la validation locale et les démonstrations ; il
//! n'implémente volontairement ni sessions ni flux SSE.

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const TOOLS: &str = r#"[
  {"name":"echo","description":"Répète le texte fourni","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]},"annotations":{"readOnlyHint":true}},
  {"name":"add","description":"Somme deux nombres","inputSchema":{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}},
  {"name":"fail","description":"Échoue toujours","inputSchema":{"type":"object"}},
  {"name":"hello","description":"Saluer (sans paramètre)","inputSchema":{"type":"object"}},
  {"name":"delete_everything","description":"Supprime des ressources","inputSchema":{"type":"object","properties":{"confirm":{"type":"boolean"}}}}
]"#;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(18765);
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind 127.0.0.1");
    println!("Serveur MCP de démonstration sur http://127.0.0.1:{port}/mcp");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            break;
        };
        tokio::spawn(async move {
            let _ = handle(stream).await;
        });
    }
}

async fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let Some((method, path, body)) = read_request(&mut stream).await? else {
        return Ok(());
    };
    if method != "POST" || !path.starts_with("/mcp") {
        return respond(&mut stream, 405, None).await;
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let rpc_method = request.get("method").and_then(Value::as_str).unwrap_or("");
    eprintln!("requête MCP reçue : {rpc_method} (id={id})");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    if rpc_method == "notifications/initialized" {
        return respond(&mut stream, 202, None).await;
    }

    let response = match rpc_method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "ih-demo-mcp", "version": "1.0.0"}
            }
        }),
        "tools/list" => {
            let tools: Value = serde_json::from_str(TOOLS).unwrap_or(json!([]));
            json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}})
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            match name {
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
                "delete_everything" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": "supprimé (démo)"}]}
                }),
                "hello" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {"content": [{"type": "text", "text": "bonjour"}]}
                }),
                "fail" => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "échec volontaire (démo)"}
                }),
                other => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32602, "message": format!("outil inconnu : {other}")}
                }),
            }
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("méthode inconnue : {rpc_method}")}
        }),
    };
    let payload = serde_json::to_vec(&response).unwrap_or_default();
    respond(&mut stream, 200, Some(payload)).await
}

async fn read_request(
    stream: &mut TcpStream,
) -> std::io::Result<Option<(String, String, Vec<u8>)>> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
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
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(Some((method, path, body)))
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    body: Option<Vec<u8>>,
) -> std::io::Result<()> {
    let body = body.unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(&body).await?;
    }
    stream.flush().await?;
    Ok(())
}
