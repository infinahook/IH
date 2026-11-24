//! Client MCP d'IH.
//!
//! Une seule crate pour les transports réseau du protocole Model Context
//! Protocol :
//!
//! | Transport | Bibliothèque | État |
//! |---|---|---|
//! | Streamable HTTP | [`rmcp`] (SDK officiel, Apache-2.0) | implémenté |
//! | SSE historique (HTTP+SSE, spécification 2024-11-05) | maison, précisément borné | implémenté |
//! | stdio local | `ih-extensions` (existant) | hors de cette crate |
//!
//! Le SDK officiel a retiré le transport SSE historique (« remove SSE transport
//! support » dans son journal des versions) : le client SSE est donc écrit ici,
//! durablement borné (tailles, délais, reconnexions), mais le **protocole**
//! JSON-RPC lui-même n'est pas réimplémenté pour Streamable HTTP.
//!
//! # Modèle de sécurité
//!
//! - toute URL passe par [`ih_security::net::validate_url`] puis les adresses
//!   résolues sont revalidées ([`ih_security::net::check_resolved`]) et
//!   épinglées dans le client HTTP (anti-DNS-rebinding) ;
//! - les redirections HTTP ne sont pas suivies (`redirect::Policy::none`) et
//!   les proxys d'environnement sont ignorés (`no_proxy`) ;
//! - les réponses sont bornées en taille avant décodage JSON ;
//! - l'URL et les en-têtes personnalisés sont validés ; `Authorization` et
//!   `Cookie` ne peuvent pas transiter par les en-têtes personnalisés ;
//! - aucun secret n'est stocké ni journalisé par cette crate : le jeton est
//!   fourni en mémoire par l'appelant (trousseau côté application).

pub mod config;
pub mod http;
pub mod schema;
pub mod session;
pub mod sse;
pub mod streamable;

pub use config::{
    McpTransport, ServerConfig, DEFAULT_TIMEOUT, MAX_RESPONSE_BYTES, MAX_SSE_EVENT_BYTES,
};
pub use schema::validate_tool_schema;
pub use session::{McpSession, ToolCallResult, ToolInfo};
pub use sse::SseSession;
pub use streamable::StreamableSession;
