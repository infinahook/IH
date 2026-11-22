//! Extensions d'IH : paquets de skills versionnés et client MCP.
//!
//! - [`skills`] : manifeste strict, installation vérifiée par empreinte
//!   SHA-256, inventaire et désinstallation confinée à la racine.
//! - [`mcp`] : client JSON-RPC 2.0 minimal pour les serveurs MCP en transport
//!   stdio (NDJSON, un objet JSON par ligne, sans `Content-Length`).
//!
//! Aucune de ces opérations ne fait d'accès réseau : l'installation copie des
//! fichiers locaux et le client MCP parle à un processus enfant.

pub mod mcp;
pub mod skills;

#[cfg(test)]
pub(crate) mod test_support;
