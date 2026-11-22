//! Modèle d'événements provider-neutre.
//!
//! Tous les adaptateurs (CLI d'agents, fournisseurs cloud, MCP) normalisent
//! leur sortie dans ce modèle. L'interface ne consomme que ces événements,
//! jamais le format brut d'un fournisseur.

use serde::{Deserialize, Serialize};

/// Flux de sortie d'un processus ou d'un appel réseau.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// État du cycle de vie d'une session d'agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Starting,
    Ready,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl AgentState {
    /// Un état terminal ne peut plus recevoir de sortie.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Transition autorisée ? Empêche les états incohérents côté serveur.
    pub fn can_transition_to(self, next: Self) -> bool {
        if self.is_terminal() {
            return false;
        }
        !matches!(
            (self, next),
            (Self::Starting, Self::Completed) | (Self::Starting, Self::WaitingApproval)
        )
    }
}

/// État d'une tâche de l'orchestrateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Ready,
    Running,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Événement émis par le noyau vers l'interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreEvent {
    AgentOutput {
        session_id: String,
        stream: OutputStream,
        seq: u64,
        data: String,
    },
    AgentState {
        session_id: String,
        state: AgentState,
    },
    TaskState {
        task_id: String,
        state: TaskState,
    },
    ProviderDelta {
        request_id: String,
        delta: String,
    },
    PermissionAsked {
        approval_id: String,
        /// `shell`, `write`, `network`, `mcp_tool`, `install`
        category: String,
        summary: String,
        detail: String,
    },
    UpdateAvailable {
        version: String,
    },
    Log {
        level: String,
        message: String,
    },
}

impl CoreEvent {
    /// Nom d'événement Tauri, stable pour le routage côté React.
    pub fn name(&self) -> &'static str {
        match self {
            Self::AgentOutput { .. } => "ih://agent-output",
            Self::AgentState { .. } => "ih://agent-state",
            Self::TaskState { .. } => "ih://task-state",
            Self::ProviderDelta { .. } => "ih://provider-delta",
            Self::PermissionAsked { .. } => "ih://permission-asked",
            Self::UpdateAvailable { .. } => "ih://update-available",
            Self::Log { .. } => "ih://log",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etats_terminaux_bloquent_les_transitions() {
        assert!(AgentState::Running.can_transition_to(AgentState::Completed));
        assert!(!AgentState::Completed.can_transition_to(AgentState::Running));
        assert!(!AgentState::Starting.can_transition_to(AgentState::Completed));
        assert!(AgentState::Starting.can_transition_to(AgentState::Ready));
    }

    #[test]
    fn serialisation_avec_etiquette_kind() {
        let ev = CoreEvent::AgentState {
            session_id: "ses-1".into(),
            state: AgentState::Running,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "agent_state");
        assert_eq!(json["state"], "running");
        assert_eq!(ev.name(), "ih://agent-state");
    }

    #[test]
    fn sortie_agent_conserve_le_numero_de_sequence() {
        let ev = CoreEvent::AgentOutput {
            session_id: "ses-1".into(),
            stream: OutputStream::Stderr,
            seq: 42,
            data: "boom".into(),
        };
        let round: CoreEvent = serde_json::from_value(serde_json::to_value(&ev).unwrap()).unwrap();
        assert_eq!(round, ev);
    }
}
