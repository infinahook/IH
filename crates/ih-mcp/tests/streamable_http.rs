//! Tests d'intégration du transport Streamable HTTP contre un vrai serveur
//! MCP local (TCP 127.0.0.1, HTTP/1.1) — aucun mock du transport.

mod support;

use ih_mcp::config::McpTransport;
use ih_mcp::session::McpSession;
use ih_mcp::ServerConfig;
use serde_json::json;
use std::time::Duration;

fn config(url: &str) -> ServerConfig {
    let mut config = ServerConfig::new("test-streamable", url, McpTransport::StreamableHttp);
    config.name = "Serveur de test".into();
    config.timeout = Duration::from_secs(5);
    config
}

#[tokio::test]
async fn connexion_decouverte_et_appels() {
    let server = support::start(support::Options::default()).await;
    let mut session = McpSession::connect(config(&server.mcp_url()))
        .await
        .expect("connexion Streamable HTTP");

    let tools = session.list_tools().await.expect("tools/list");
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert!(names.contains(&"echo"), "outils : {names:?}");
    assert!(names.contains(&"delete_everything"));

    let echo = tools.iter().find(|tool| tool.name == "echo").unwrap();
    assert_eq!(echo.input_schema["type"], "object");
    // Classification locale, jamais fournie par le serveur.
    assert_eq!(
        echo.safety.category,
        ih_security::mcp::McpToolCategory::Read
    );

    let danger = tools
        .iter()
        .find(|tool| tool.name == "delete_everything")
        .unwrap();
    assert_eq!(
        danger.safety.category,
        ih_security::mcp::McpToolCategory::Destructive
    );
    assert_eq!(danger.safety.risk, ih_security::risk::RiskLevel::Critical);
    assert!(!danger.safety.enabled_by_default);

    let result = session
        .call_tool("echo", json!({ "text": "bonjour" }))
        .await
        .expect("appel echo");
    assert!(!result.is_error);
    assert_eq!(result.text, "bonjour");

    let sum = session
        .call_tool("add", json!({ "a": 2, "b": 3 }))
        .await
        .expect("appel add");
    assert_eq!(sum.text, "5");

    session.close().await.expect("fermeture");
    server.shutdown().await;
}

#[tokio::test]
async fn erreur_jsonrpc_du_serveur_remontee_sans_tuer_la_session() {
    let server = support::start(support::Options::default()).await;
    let mut session = McpSession::connect(config(&server.mcp_url()))
        .await
        .unwrap();
    let error = session
        .call_tool("fail", json!({}))
        .await
        .expect_err("l'outil fail doit échouer");
    let message = error.to_string();
    assert!(message.contains("-32601"), "{message}");
    assert!(message.contains("outil introuvable"), "{message}");

    // La session reste utilisable après une erreur applicative.
    let echo = session
        .call_tool("echo", json!({"text": "ok"}))
        .await
        .unwrap();
    assert_eq!(echo.text, "ok");
    session.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn jeton_bearer_exige_par_le_serveur() {
    let options = support::Options {
        bearer: Some("secret-token".into()),
        ..Default::default()
    };
    let server = support::start(options).await;

    // Sans jeton : refus explicite.
    let error = McpSession::connect(config(&server.mcp_url()))
        .await
        .expect_err("401 attendu sans jeton");
    assert!(
        error.to_string().contains("refusée") || error.to_string().contains("HTTP 401"),
        "{error}"
    );

    // Avec jeton : succès.
    let mut secured = config(&server.mcp_url());
    secured.bearer = Some("secret-token".into());
    let mut session = McpSession::connect(secured)
        .await
        .expect("connexion avec jeton");
    let tools = session.list_tools().await.unwrap();
    assert!(!tools.is_empty());
    session.close().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn timeout_sur_outil_lent() {
    let options = support::Options {
        delay_ms: 1_500,
        ..Default::default()
    };
    let server = support::start(options).await;
    let mut session = ServerConfig::new("slow", server.mcp_url(), McpTransport::StreamableHttp);
    session.timeout = Duration::from_millis(300);
    let mut session = McpSession::connect(session).await.unwrap();

    let started = std::time::Instant::now();
    let error = session.call_tool("slow", json!({})).await.unwrap_err();
    assert!(error.to_string().contains("délai"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(4));
    server.shutdown().await;
}

#[tokio::test]
async fn redirection_http_non_suivie() {
    let server = support::start(support::Options::default()).await;
    server.mode.lock().await.redirect = true;
    let result = McpSession::connect(config(&server.mcp_url())).await;
    assert!(
        result.is_err(),
        "une redirection doit faire échouer la connexion"
    );
    // Aucune requête vers la cible de redirection : le client épinglé ne suit
    // pas les 302 (reqwest redirect::Policy::none).
    let requests = server.requests.lock().await.clone();
    assert!(
        requests.iter().all(|entry| !entry.contains("/evil")),
        "redirection suivie : {requests:?}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn reponse_trop_volumineuse_refusee() {
    let server = support::start(support::Options::default()).await;
    server.mode.lock().await.oversized = true;
    let mut session = McpSession::connect(config(&server.mcp_url()))
        .await
        .unwrap();
    let error = session
        .call_tool("big", json!({}))
        .await
        .expect_err("une réponse de 9 Mio doit être refusée");
    assert!(!error.to_string().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn erreur_serveur_propre_sur_tools_list_invalide() {
    let server = support::start(support::Options::default()).await;
    server.mode.lock().await.bad_json = true;
    let mut session = McpSession::connect(config(&server.mcp_url()))
        .await
        .unwrap();
    let error = session
        .list_tools()
        .await
        .expect_err("un JSON tronqué doit être refusé");
    assert!(!error.to_string().is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn outils_a_schema_invalide_exclus_du_catalogue() {
    let mut options = support::Options::default();
    options.tools.push(support::TestTool {
        name: "broken".into(),
        description: "schéma invalide".into(),
        input_schema: json!({"type": "array"}),
        annotations: serde_json::Value::Null,
    });
    let server = support::start(options).await;
    let mut session = McpSession::connect(config(&server.mcp_url()))
        .await
        .unwrap();
    let tools = session.list_tools().await.unwrap();
    assert!(
        tools.iter().all(|tool| tool.name != "broken"),
        "un schéma non-object doit être exclu"
    );
    assert!(tools.iter().any(|tool| tool.name == "echo"));
    server.shutdown().await;
}

#[tokio::test]
async fn arguments_non_objets_refuses_avant_reseau() {
    let server = support::start(support::Options::default()).await;
    let mut session = McpSession::connect(config(&server.mcp_url()))
        .await
        .unwrap();
    let error = session
        .call_tool("echo", json!(["pas", "un", "objet"]))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("objet JSON"), "{error}");
    server.shutdown().await;
}
