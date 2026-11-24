//! Tests d'intégration du transport SSE historique contre un vrai serveur
//! MCP local (TCP 127.0.0.1) : connexion réelle, flux SSE réel, reconnexion.

mod support;

use ih_mcp::config::McpTransport;
use ih_mcp::session::McpSession;
use ih_mcp::ServerConfig;
use serde_json::json;
use std::time::Duration;

fn config(url: &str) -> ServerConfig {
    let mut config = ServerConfig::new("test-sse", url, McpTransport::Sse);
    config.name = "Serveur SSE de test".into();
    config.timeout = Duration::from_secs(5);
    config
}

#[tokio::test]
async fn connexion_sse_decouverte_et_appels() {
    let server = support::start(support::Options::default()).await;
    let mut session = McpSession::connect(config(&server.sse_url()))
        .await
        .expect("connexion SSE");

    let tools = session.list_tools().await.expect("tools/list SSE");
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    assert!(tools.iter().any(|tool| tool.name == "delete_everything"));

    let result = session
        .call_tool("echo", json!({"text": "ssé"}))
        .await
        .expect("appel echo SSE");
    assert_eq!(result.text, "ssé");
    assert!(!result.is_error);

    let error = session
        .call_tool("fail", json!({}))
        .await
        .expect_err("fail doit échouer");
    assert!(error.to_string().contains("-32601"), "{error}");

    session.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn reconnexion_sse_apres_coupure_du_flux() {
    let server = support::start(support::Options::default()).await;
    let mut session = McpSession::connect(config(&server.sse_url()))
        .await
        .unwrap();
    assert!(session
        .list_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "echo"));

    // Le serveur coupe le flux ; le client doit se reconnecter (nouveau GET
    // /sse) et rejouer le handshake sur la nouvelle session.
    server.drop_streams.notify_waiters();
    tokio::time::sleep(Duration::from_millis(900)).await;

    let gets = server
        .requests
        .lock()
        .await
        .iter()
        .filter(|entry| entry.starts_with("GET /sse"))
        .count();
    assert!(gets >= 2, "le client doit rouvrir un flux SSE (GET /sse)");

    // Un appel après reconnexion doit réussir.
    let mut last_error = String::new();
    for _ in 0..20 {
        match session
            .call_tool("echo", json!({"text": "après reconnexion"}))
            .await
        {
            Ok(result) => {
                assert_eq!(result.text, "après reconnexion");
                last_error.clear();
                break;
            }
            Err(error) => {
                last_error = error.to_string();
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    }
    assert!(
        last_error.is_empty(),
        "appel après reconnexion en échec : {last_error}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn jeton_bearer_sse_exige() {
    let options = support::Options {
        bearer: Some("sse-token".into()),
        ..Default::default()
    };
    let server = support::start(options).await;

    let error = McpSession::connect(config(&server.sse_url()))
        .await
        .expect_err("401 attendu sans jeton");
    assert!(
        error.to_string().contains("401")
            || error.to_string().contains("endpoint")
            || error.to_string().contains("refusé"),
        "{error}"
    );

    let mut secured = config(&server.sse_url());
    secured.bearer = Some("sse-token".into());
    let mut session = McpSession::connect(secured)
        .await
        .expect("connexion avec jeton");
    assert!(session
        .list_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "echo"));
    session.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn timeout_sse_sur_outil_lent() {
    let options = support::Options {
        delay_ms: 1_500,
        ..Default::default()
    };
    let server = support::start(options).await;
    let mut slow = config(&server.sse_url());
    slow.timeout = Duration::from_millis(300);
    // Le handshake lui-même dépasse le délai : deux issues acceptables et
    // explicites selon le moment où la requête expire.
    match McpSession::connect(slow).await {
        Err(error) => assert!(error.to_string().contains("délai"), "{error}"),
        Ok(mut session) => {
            let error = session.call_tool("slow", json!({})).await.unwrap_err();
            assert!(error.to_string().contains("délai"), "{error}");
        }
    }
    server.shutdown().await;
}
