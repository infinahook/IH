//! Sécurité d'IH : classification du risque, approbations explicites et
//! stockage des secrets.
//!
//! Deux invariants :
//! 1. aucune action sensible ne s'exécute sans une approbation consommée une
//!    seule fois et non expirée ;
//! 2. aucun secret n'est jamais écrit dans un fichier de configuration, une
//!    base SQLite ou un journal : le trousseau du système est la seule voie.

pub mod approval;
pub mod risk;
pub mod secrets;

pub use approval::{
    ApprovalDecision, ApprovalRegistry, ApprovalRequest, ApprovalState, ApprovalTicket,
};
pub use risk::{classify_command, CommandAssessment, RiskLevel};
pub use secrets::{KeyringSecretStore, MemorySecretStore, SecretStore, SecretStoreStatus};
