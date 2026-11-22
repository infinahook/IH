//! Orchestration : graphe de tâches, équipes multi-agents, ordonnancement et
//! injection de la mémoire.

pub mod graph;
pub mod memory;
pub mod scheduler;
pub mod team;

pub use graph::{TaskGraph, TaskNode, TeamRole};
pub use memory::{build_context, estimate_tokens, MemorySnippet, DEFAULT_MEMORY_BUDGET};
pub use scheduler::Scheduler;
pub use team::{complete, plan_goal, Plan, Team, TeamMember};
