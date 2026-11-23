//! Mode Équipe : Lead → Workers → Reviewer, exécuté en sessions d'agent réelles.
//!
//! Le plan vient de `ih-orchestrator` (graphe de tâches et rôles), les sessions
//! de `ih-agents`, l'isolation de `ih-agent-mode` (worktree), et les
//! modifications passent par le système de diff existant.
//!
//! Une seule approbation couvre la course entière, liée à une empreinte
//! (objectif + agents + répertoire). Chaque tâche s'exécute dans le **même**
//! worktree isolé : le projet n'est jamais modifié pendant la course.

use super::project_path;
use crate::state::{AppState, PendingDiff};
use ih_agent_mode::{collect_changes, create_sandbox, remove_sandbox, MAX_CHANGE_BYTES};
use ih_core::events::TaskState;
use ih_core::TaskId;
use ih_core::{IhError, IhResult, ProjectId};
use ih_orchestrator::{plan_goal, Team, TeamRole};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

const TEAM_CATEGORY: &str = "agent_team";

#[derive(Debug, Clone, Deserialize)]
pub struct TeamRequest {
    pub project_id: String,
    pub goal: String,
    /// Agent coordinateur (analyse + décomposition).
    pub lead: String,
    /// Exécutants (1 à 3).
    pub workers: Vec<String>,
    /// Relecteur (optionnel : le lead par défaut).
    pub reviewer: Option<String>,
    /// Nombre de tâches d'exécution (1 à 4).
    #[serde(default = "default_workers")]
    pub work_items: usize,
}

fn default_workers() -> usize {
    2
}

/// Empreinte de la course : toute modification invalide l'approbation.
fn fingerprint(request: &TeamRequest, root: &std::path::Path) -> IhResult<String> {
    Ok(ih_agent_mode::team_fingerprint(
        &request.goal,
        &request.lead,
        &request.workers,
        request.reviewer.as_deref(),
        root,
    ))
}

fn build_team(request: &TeamRequest, config: &ih_core::config::AppConfig) -> IhResult<Team> {
    let mut team = Team::new("ih");
    team.max_parallel = 1; // séquentiel : la relecture dépend des exécutions
    for (label, id) in std::iter::once(("lead", &request.lead))
        .chain(request.workers.iter().map(|w| ("worker", w)))
        .chain(request.reviewer.as_ref().map(|r| ("reviewer", r)))
    {
        if config.agent(id).is_none() {
            return Err(IhError::NotFound(format!("agent {id} ({label})")));
        }
        team = team.with_member(id.clone(), TeamRole::parse(label)?);
    }
    team.validate()?;
    Ok(team)
}

#[derive(Debug, Serialize)]
pub struct TeamApprovalView {
    pub approval_id: String,
    pub detail: String,
    pub fingerprint: String,
    pub plan_summary: String,
}

/// Prépare l'approbation d'une course d'équipe (affiche le plan prévu).
#[tauri::command]
pub fn team_request_approval(
    state: State<'_, AppState>,
    request: TeamRequest,
) -> IhResult<TeamApprovalView> {
    if request.goal.trim().is_empty() {
        return Err(IhError::Invalid("objectif vide".into()));
    }
    let root = project_path(&state, request.project_id.as_str())?;
    let config = state.config_snapshot()?;
    let team = build_team(&request, &config)?;
    let plan = plan_goal(&request.goal, &team, request.work_items)?;
    plan.graph.validate()?;

    let fingerprint = fingerprint(&request, &root)?;
    let summary = plan
        .graph
        .topological_order()
        .iter()
        .map(|node| format!("- [{}] {}", node.role.as_str(), node.title))
        .collect::<Vec<_>>()
        .join("\n");
    let detail = format!(
        "Objectif : {}\nRépertoire isolé sous : {}\nEmpreinte : {fingerprint}\n\nPlan :\n{summary}\n\n\
         Chaque tâche s'exécute dans un worktree git isolé ; le projet n'est pas modifié. \
         Les fichiers produits seront présentés en diff avant application.",
        request.goal,
        root.display()
    );
    let approval = state.approvals.ask(
        TEAM_CATEGORY,
        &format!("Lancer une équipe sur : {}", truncate(&request.goal, 100)),
        &detail,
        ih_security::risk::RiskLevel::High,
    )?;

    Ok(TeamApprovalView {
        approval_id: approval.id.into_string(),
        detail,
        fingerprint,
        plan_summary: summary,
    })
}

#[derive(Debug, Serialize)]
pub struct TeamTaskResult {
    pub task_id: String,
    pub title: String,
    pub role: String,
    pub state: String,
    pub sessions: usize,
}

#[derive(Debug, Serialize)]
pub struct TeamRunResult {
    pub run_id: String,
    pub tasks: Vec<TeamTaskResult>,
    pub files: Vec<String>,
    pub changes_prepared: usize,
    pub summary: String,
}

/// Exécute la course : une session d'agent par tâche, dans un worktree unique.
#[tauri::command(async)]
pub fn team_run(
    state: State<'_, AppState>,
    request: TeamRequest,
    approval_id: String,
) -> IhResult<TeamRunResult> {
    if request.goal.trim().is_empty() {
        return Err(IhError::Invalid("objectif vide".into()));
    }
    let project = ProjectId::parse(request.project_id.clone())?;
    let root = project_path(&state, request.project_id.as_str())?;
    let config = state.config_snapshot()?;
    let team = build_team(&request, &config)?;

    // Approbation : doit correspondre exactement à cette course.
    let expected = fingerprint(&request, &root)?;
    let pending = state
        .approvals
        .get(&approval_id)?
        .ok_or_else(|| IhError::NotFound(format!("approbation {approval_id}")))?;
    if !ih_agent_mode::approval_covers(&pending.detail, &expected) {
        return Err(IhError::Denied(
            "l'approbation ne correspond pas à cet objectif ou à ces agents".into(),
        ));
    }
    state.approvals.consume(&approval_id, TEAM_CATEGORY)?;

    let plan = plan_goal(&request.goal, &team, request.work_items)?;
    plan.graph.validate()?;
    let run_id = format!("team-{}", uuid::Uuid::new_v4());
    let sandbox = create_sandbox(&root, &state.paths.data_dir().join("worktrees"), &run_id)?;

    // Tâches persistées d'abord, puis exécution réelle (une session par tâche).
    let mut store_ids: Vec<(String, String)> = Vec::new();
    for node in plan.graph.topological_order() {
        let store_task = state.store.create_task(&ih_store::NewTask {
            project_id: project.clone(),
            title: node.title.clone(),
            description: node.description.clone(),
            assignee: node.assignee.clone(),
            depends_on: Vec::new(),
            role: node.role.as_str().to_string(),
        })?;
        store_ids.push((node.id.to_string(), store_task.id.to_string()));
    }

    let max_timeout = config
        .agents
        .iter()
        .map(|a| a.timeout_secs)
        .max()
        .unwrap_or(1_800);
    let app = state.app_handle.clone();
    let run_for_events = run_id.clone();
    let mut emit = |outcome: &ih_agent_mode::TeamTaskOutcome| {
        let _ = app.emit(
            "ih://team-event",
            serde_json::json!({
                "run_id": run_for_events,
                "task_id": outcome.task_id,
                "title": outcome.title,
                "role": outcome.role,
                "state": outcome.state,
            }),
        );
    };
    let outcomes = ih_agent_mode::run_team_tasks_with_events(
        &state.supervisor,
        &sandbox,
        &config,
        &plan,
        &request.goal,
        std::time::Duration::from_secs(max_timeout + 30),
        &mut emit,
    )?;

    let mut results: Vec<TeamTaskResult> = Vec::new();
    for outcome in &outcomes {
        let task_id = store_ids
            .iter()
            .find(|(node_id, _)| node_id == &outcome.task_id)
            .map(|(_, id)| id.clone())
            .unwrap_or_default();
        let task_state = match outcome.state.as_str() {
            "completed" => TaskState::Completed,
            "cancelled" => TaskState::Cancelled,
            _ => TaskState::Failed,
        };
        if let Ok(parsed) = TaskId::parse(&task_id) {
            let _ = state.store.update_task_state(&parsed, task_state);
        }
        let _ = state.app_handle.emit(
            "ih://team-event",
            serde_json::json!({
                "run_id": run_id,
                "task_id": outcome.task_id,
                "title": outcome.title,
                "role": outcome.role,
                "state": outcome.state,
            }),
        );
        results.push(TeamTaskResult {
            task_id: outcome.task_id.clone(),
            title: outcome.title.clone(),
            role: outcome.role.clone(),
            state: outcome.state.clone(),
            sessions: if outcome.state == "cancelled" { 0 } else { 1 },
        });
    }
    let aborted: Option<String> = outcomes
        .iter()
        .find(|o| o.state != "completed")
        .map(|o| format!("tâche « {} » : {}", o.title, o.state));

    // Modifications produites par l'équipe, préparées comme diffs en attente.
    let mut files = Vec::new();
    let mut prepared = 0usize;
    if let Ok(changes) = collect_changes(&sandbox, MAX_CHANGE_BYTES) {
        for change in &changes {
            if change.binary || change.too_large {
                continue;
            }
            files.push(change.path.clone());
            let before_sha = ih_workspace::fs::ProjectFs::new(&root)
                .ok()
                .and_then(|fs| fs.metadata(&change.path).ok())
                .map(|meta| meta.sha256);
            if let Ok(mut guard) = state.pending_diffs.lock() {
                guard.insert(
                    format!("diff-{}", uuid::Uuid::new_v4()),
                    PendingDiff {
                        project_id: project.to_string(),
                        path: change.path.clone(),
                        before_sha256: before_sha,
                        content: change.after.clone().unwrap_or_default(),
                        delete: change.after.is_none(),
                        origin: format!("team:{run_id}"),
                        created_at: chrono::Utc::now(),
                    },
                );
                prepared += 1;
            }
        }
    }
    let _ = remove_sandbox(&sandbox);

    let summary = match (&aborted, prepared) {
        (Some(reason), _) => format!("course interrompue : {reason}"),
        (None, 0) => "course terminée — aucun fichier modifié".to_string(),
        (None, n) => format!("course terminée — {n} fichier(s) en attente de validation"),
    };
    let _ = state.app_handle.emit(
        "ih://team-event",
        serde_json::json!({
            "run_id": run_id,
            "state": if aborted.is_some() { "failed" } else { "completed" },
            "summary": summary,
            "files": files,
        }),
    );

    Ok(TeamRunResult {
        run_id,
        tasks: results,
        files,
        changes_prepared: prepared,
        summary,
    })
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}
