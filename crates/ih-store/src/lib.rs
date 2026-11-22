//! Persistance SQLite d'IH.
//!
//! Ce crate possède l'unique base SQLite locale de l'application. Il ne
//! contient aucune donnée de démonstration : les lignes ne sont créées que par
//! les appels explicites de l'orchestrateur, de l'interface ou des tests.
//!
//! # Garanties
//!
//! - **WAL** et `foreign_keys = ON` à chaque ouverture ;
//! - migrations versionnées par `PRAGMA user_version`, appliquées
//!   atomiquement (la version attendue est exposée par [`SCHEMA_VERSION`]) ;
//! - suppression en cascade des données IH d'un projet, sans jamais toucher
//!   aux fichiers du projet sur le disque ;
//! - une connexion partagée derrière un `Mutex`, d'où un `Store: Send + Sync`
//!   utilisable dans un état Tauri ;
//! - erreurs SQLite converties en [`ih_core::IhError::Db`] avec contexte.
//!
//! # Recherche mémoire
//!
//! `search_memory` utilise exclusivement des paramètres liés. Les caractères
//! `%`, `_` et `\` saisis par l'utilisateur sont échappés : la recherche reste
//! littérale. `LIKE` SQLite est insensible à la casse pour l'ASCII ; les
//! accents ne sont pas repliés faute d'extension ICU dans la compilation
//! `bundled`.

mod migrate;
mod model;
mod store;
#[cfg(test)]
mod tests;
mod util;

pub use migrate::SCHEMA_VERSION;
pub use model::{
    AgentEventRow, AgentSessionDetail, AuditEntry, InstalledExtension, MemoryEntry, Message,
    NewAgentEvent, NewAuditEntry, NewMemoryEntry, NewMessage, NewSession, NewTask, NewWorkflowRun,
    Project, SessionRow, Task, Thread, Workflow, WorkflowRun,
};
pub use store::Store;
