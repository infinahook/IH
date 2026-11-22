//! Runtime d'agents : adaptateurs CLI et supervision de processus.

pub mod adapters;
pub mod supervisor;

pub use adapters::{AgentAdapter, AgentInfo, AgentRegistry};
pub use supervisor::{AgentSupervisor, EventSink, SessionSnapshot, StartRequest};
