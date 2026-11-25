//! Runtime d'agents : adaptateurs CLI et supervision de processus.

pub mod adapters;
pub mod normalize;
pub mod sandbox;
pub mod supervisor;

pub use adapters::{AgentAdapter, AgentInfo, AgentRegistry};
pub use normalize::normalize_line;
pub use supervisor::{AgentSupervisor, EventSink, SessionSnapshot, StartRequest};
