//! Noyau d'IH : types partagés, configuration, politique de ressources et
//! plateforme. Aucun autre crate ne doit redéfinir ces contrats.

pub mod config;
pub mod error;
pub mod events;
pub mod ids;
pub mod paths;
pub mod platform;
pub mod redaction;
pub mod resources;

pub use error::{IhError, IhResult};
pub use events::{AgentState, CoreEvent, OutputStream, TaskState};
pub use ids::{AgentId, ApprovalId, MessageId, ProjectId, ServerId, SessionId, TaskId, ThreadId};
pub use paths::AppPaths;
pub use resources::ResourcePolicy;

/// Version applicative unique, partagée par le noyau et l'interface.
pub const APP_NAME: &str = "IH";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
