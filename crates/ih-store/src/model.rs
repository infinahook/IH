//! Types publics persistés par `ih-store`.
//!
//! Les horodatages sont systématiquement stockés en UTC et exposés en
//! [`chrono::DateTime<chrono::Utc>`]. Les identifiants métier réutilisent les
//! types typés de `ih-core` afin qu'aucun crate ne redéfinisse ce contrat.

use chrono::{DateTime, Utc};
use ih_core::events::{AgentState, TaskState};
use ih_core::{MessageId, ProjectId, SessionId, TaskId, ThreadId};
use serde::{Deserialize, Serialize};

/// Métadonnées IH d'un projet ouvert dans l'application.
///
/// Le dossier lui-même n'est jamais copié ni modifié par la base : seule cette
/// ligne décrit le projet pour l'interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    /// Chemin absolu du dossier du projet sur le disque.
    pub path: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_opened_at: DateTime<Utc>,
    /// Le projet a été explicitement approuvé par l'utilisateur.
    pub trusted: bool,
}

/// Conversation rattachée à un projet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub project_id: ProjectId,
    pub title: String,
    /// Identifiant de l'agent configuré, lorsqu'un agent est associé.
    pub agent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Message à insérer dans une conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMessage {
    pub thread_id: ThreadId,
    /// `user`, `assistant`, `system` ou `tool`.
    pub role: String,
    pub content: String,
    /// Charge utile arbitraire (usage d'outil, jetons, références…).
    pub metadata: Option<serde_json::Value>,
}

/// Message persisté.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

/// Session d'agent à créer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSession {
    pub project_id: ProjectId,
    pub agent_id: String,
    pub thread_id: Option<ThreadId>,
    /// Invite initiale transmise à l'agent ; conservée pour l'audit local.
    pub prompt: String,
}

/// Session d'agent persistée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: SessionId,
    pub project_id: ProjectId,
    pub agent_id: String,
    pub thread_id: Option<ThreadId>,
    pub state: AgentState,
    pub exit_code: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// Événement d'une session d'agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEventRow {
    pub id: i64,
    pub session_id: SessionId,
    pub seq: u32,
    pub kind: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Événement à enregistrer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAgentEvent {
    pub session_id: SessionId,
    pub seq: u32,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Session d'agent avec ses métadonnées d'exécution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSessionDetail {
    pub session: SessionRow,
    pub adapter: Option<String>,
    pub task: Option<String>,
    pub workdir: Option<String>,
    pub summary: Option<String>,
    pub files: Vec<String>,
}

/// Tâche à créer dans l'orchestrateur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewTask {
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub assignee: Option<String>,
    /// Tâches devant être terminées avant celle-ci.
    pub depends_on: Vec<TaskId>,
    /// `lead`, `worker` ou `reviewer`.
    pub role: String,
}

/// Tâche persistée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub title: String,
    pub description: String,
    pub state: TaskState,
    pub assignee: Option<String>,
    pub depends_on: Vec<TaskId>,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Entrée de mémoire à enregistrer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemoryEntry {
    /// Obligatoire pour `scope = "project"`, interdit pour `scope = "global"`.
    pub project_id: Option<ProjectId>,
    /// `project` ou `global`.
    pub scope: String,
    /// Clé courte optionnelle, utile pour retrouver une consigne précise.
    pub key: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub source: String,
}

/// Entrée de mémoire persistée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub project_id: Option<ProjectId>,
    pub scope: String,
    pub key: Option<String>,
    pub content: String,
    pub tags: Vec<String>,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Extension installée (skill ou plugin MCP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstalledExtension {
    pub id: String,
    /// `skill` ou `plugin`.
    pub kind: String,
    pub name: String,
    pub version: String,
    /// Provenance lisible : dépôt, URL ou `local`.
    pub source: String,
    /// Manifeste complet validé lors de l'installation.
    pub manifest: serde_json::Value,
    /// Empreinte du paquet installé, si disponible.
    pub sha256: Option<String>,
    pub enabled: bool,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Workflow planifié d'un projet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub project_id: ProjectId,
    pub name: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    /// Étapes du workflow, conservées telles quelles en JSON.
    pub steps: serde_json::Value,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Exécution de workflow à enregistrer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewWorkflowRun {
    pub workflow_id: String,
    /// Statut libre décidé par l'orchestrateur (`success`, `failed`…).
    pub status: String,
    pub detail: Option<String>,
}

/// Exécution de workflow persistée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: i64,
    pub workflow_id: String,
    pub status: String,
    pub detail: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Entrée d'audit à enregistrer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAuditEntry {
    /// Catégorie de décision (`shell`, `write`, `network`, `mcp_tool`, `install`…).
    pub category: String,
    pub summary: String,
    pub detail: Option<String>,
    /// `approved`, `denied` ou `auto`.
    pub decision: String,
}

/// Entrée d'audit persistée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub category: String,
    pub summary: String,
    pub detail: Option<String>,
    pub decision: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// MCP (serveurs distants, catalogue d'outils, appels)
// ---------------------------------------------------------------------------

/// Serveur MCP distant enregistré. Aucun secret n'est stocké ici.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerRecord {
    pub id: String,
    pub name: String,
    /// URL complète du serveur (validée avant connexion).
    pub url: String,
    /// `streamable_http` ou `sse`.
    pub transport: String,
    /// En-têtes personnalisés **non secrets** (JSON objet en base).
    pub headers: std::collections::BTreeMap<String, String>,
    pub timeout_secs: u32,
    pub enabled: bool,
    /// Pré-autorisation explicite : seules les lectures peuvent être
    /// auto-approuvées ; tout le reste exige une approbation interactive.
    pub allow_auto_approve: bool,
    /// `disconnected`, `connecting`, `connected`, `error`.
    pub last_status: String,
    pub last_error: Option<String>,
    pub last_connected_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Outil MCP découvert et mis en catalogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolRecord {
    pub server_id: String,
    pub name: String,
    pub description: String,
    /// Schéma JSON d'entrée, déjà validé structurellement.
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
    /// `read`, `write`, `execute`, `destructive`, `network`.
    pub category: String,
    /// `low`, `medium`, `high`, `critical`.
    pub risk: String,
    /// Un outil découvert n'est jamais autorisé par défaut.
    pub enabled: bool,
    pub discovered_at: DateTime<Utc>,
}

/// Appel MCP à historiser (observabilité).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMcpCall {
    pub server_id: String,
    pub tool_name: String,
    /// `ui`, `chat`, `agent`, `team`, `automation`.
    pub caller: String,
    pub caller_id: Option<String>,
    pub arguments: serde_json::Value,
    /// `completed`, `failed`, `denied`, `approval_required`.
    pub status: String,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
}

/// Appel MCP persisté.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpCallRecord {
    pub id: i64,
    pub server_id: String,
    pub tool_name: String,
    pub caller: String,
    pub caller_id: Option<String>,
    pub arguments: serde_json::Value,
    pub status: String,
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}
