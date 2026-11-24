//! Commandes tâches, mémoire, historique, automatisations et mises à jour.

use super::core::parse_task_state;
use super::project_path;
use crate::state::{AppState, UpdateStatus};
use ih_core::{IhError, IhResult, ProjectId};
use ih_orchestrator::{plan_goal, MemorySnippet, Team, TeamRole};
use ih_security::risk::RiskLevel;
use ih_store::{
    MemoryEntry, Message, NewMemoryEntry, NewTask, Task, Thread, Workflow, WorkflowRun,
};
use serde::Serialize;
use std::time::Duration;
use tauri::State;

// ------------------------------------------------------------------- tâches

#[tauri::command]
pub fn tasks_list(state: State<'_, AppState>, project_id: String) -> IhResult<Vec<Task>> {
    let id = ProjectId::parse(project_id)?;
    state.store.list_tasks(&id)
}

#[tauri::command]
pub fn task_create(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
    description: Option<String>,
    role: Option<String>,
    assignee: Option<String>,
) -> IhResult<Task> {
    let project = ProjectId::parse(project_id)?;
    let role = match role.as_deref() {
        Some(value) => TeamRole::parse(value)?.as_str().to_string(),
        None => "worker".to_string(),
    };
    state.store.create_task(&NewTask {
        project_id: project,
        title,
        description: description.unwrap_or_default(),
        assignee,
        depends_on: Vec::new(),
        role,
    })
}

#[tauri::command]
pub fn task_set_state(state: State<'_, AppState>, id: String, task_state: String) -> IhResult<()> {
    let id = ih_core::TaskId::parse(id)?;
    let parsed = parse_task_state(&task_state)?;
    state.store.update_task_state(&id, parsed)
}

#[derive(Debug, Serialize)]
pub struct PlanView {
    pub goal: String,
    pub tasks: Vec<Task>,
}

#[tauri::command]
pub fn plan_create(
    state: State<'_, AppState>,
    project_id: String,
    goal: String,
    workers: Option<usize>,
) -> IhResult<PlanView> {
    let project = ProjectId::parse(project_id)?;
    project_path(&state, project.as_str())?;
    let config = state.config_snapshot()?;
    let agents: Vec<String> = config
        .agents
        .iter()
        .filter(|a| a.enabled)
        .map(|a| a.id.clone())
        .collect();
    if agents.is_empty() {
        return Err(IhError::Config("aucun agent activé".into()));
    }
    let mut team = Team::new("ih");
    team = team.with_member(agents[0].clone(), TeamRole::Lead);
    for agent in agents.iter().skip(1) {
        team = team.with_member(agent.clone(), TeamRole::Worker);
    }
    if agents.len() > 1 {
        team = team.with_member(agents[1].clone(), TeamRole::Reviewer);
    }
    team.max_parallel = config.resources.max_concurrent_agents;

    let plan = plan_goal(&goal, &team, workers.unwrap_or(2))?;
    let mut created: Vec<Task> = Vec::new();
    let mut mapping: std::collections::HashMap<String, ih_core::TaskId> =
        std::collections::HashMap::new();

    for node in plan.graph.iter() {
        let depends_on: Vec<ih_core::TaskId> = node
            .depends_on
            .iter()
            .filter_map(|dep| mapping.get(dep.as_str()).cloned())
            .collect();
        let task = state.store.create_task(&NewTask {
            project_id: project.clone(),
            title: node.title.clone(),
            description: node.description.clone(),
            assignee: node.assignee.clone(),
            depends_on,
            role: node.role.as_str().to_string(),
        })?;
        mapping.insert(node.id.to_string(), task.id.clone());
        created.push(task);
    }
    Ok(PlanView {
        goal,
        tasks: created,
    })
}

// ------------------------------------------------------------------ mémoire

#[tauri::command]
pub fn memory_list(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> IhResult<Vec<MemoryEntry>> {
    let project = project_id.map(ProjectId::parse).transpose()?;
    state.store.list_memory(project.as_ref())
}

#[tauri::command]
pub fn memory_add(
    state: State<'_, AppState>,
    project_id: Option<String>,
    content: String,
    tags: Option<Vec<String>>,
) -> IhResult<MemoryEntry> {
    if content.trim().is_empty() {
        return Err(IhError::Invalid("contenu de mémoire vide".into()));
    }
    let project = project_id.map(ProjectId::parse).transpose()?;
    let scope = if project.is_some() {
        "project"
    } else {
        "global"
    };
    state.store.add_memory(&NewMemoryEntry {
        project_id: project,
        scope: scope.to_string(),
        key: None,
        content,
        tags: tags.unwrap_or_default(),
        source: "utilisateur".into(),
    })
}

#[tauri::command]
pub fn memory_search(
    state: State<'_, AppState>,
    project_id: Option<String>,
    query: String,
) -> IhResult<Vec<MemoryEntry>> {
    let project = project_id.map(ProjectId::parse).transpose()?;
    state.store.search_memory(project.as_ref(), &query, 50)
}

#[tauri::command]
pub fn memory_delete(state: State<'_, AppState>, id: String) -> IhResult<()> {
    state.store.delete_memory(&id)
}

#[tauri::command]
pub fn memory_preview(state: State<'_, AppState>, project_id: Option<String>) -> IhResult<String> {
    let project = project_id.map(ProjectId::parse).transpose()?;
    let entries = state.store.list_memory(project.as_ref())?;
    let snippets: Vec<MemorySnippet> = entries
        .into_iter()
        .map(|entry| MemorySnippet {
            content: entry.content,
            source: entry.source,
        })
        .collect();
    ih_orchestrator::build_context(&snippets, ih_orchestrator::DEFAULT_MEMORY_BUDGET)
}

// ---------------------------------------------------------------- historique

#[tauri::command]
pub fn threads_list(state: State<'_, AppState>, project_id: String) -> IhResult<Vec<Thread>> {
    let id = ProjectId::parse(project_id)?;
    state.store.list_threads(&id)
}

#[tauri::command]
pub fn thread_create(
    state: State<'_, AppState>,
    project_id: String,
    title: String,
) -> IhResult<Thread> {
    let id = ProjectId::parse(project_id)?;
    state.store.create_thread(&id, &title, None)
}

#[tauri::command]
pub fn messages_list(
    state: State<'_, AppState>,
    thread_id: String,
    limit: Option<u32>,
    offset: Option<u32>,
) -> IhResult<Vec<Message>> {
    let id = ih_core::ThreadId::parse(thread_id)?;
    state
        .store
        .list_messages(&id, limit.unwrap_or(50), offset.unwrap_or(0))
}

#[tauri::command]
pub fn thread_delete(state: State<'_, AppState>, id: String) -> IhResult<()> {
    let id = ih_core::ThreadId::parse(id)?;
    state.store.delete_thread(&id)
}

#[derive(Serialize)]
pub struct HistoryExport {
    pub project: String,
    pub exported_at: String,
    pub threads: Vec<ThreadExport>,
}

#[derive(Serialize)]
pub struct ThreadExport {
    pub thread: Thread,
    pub messages: Vec<Message>,
}

#[tauri::command]
pub fn history_export(state: State<'_, AppState>, project_id: String) -> IhResult<String> {
    let id = ProjectId::parse(project_id)?;
    let project = state
        .store
        .get_project(&id)?
        .ok_or_else(|| IhError::NotFound("projet".into()))?;
    let threads = state.store.list_threads(&id)?;
    let mut exported = Vec::new();
    for thread in threads {
        let messages = state.store.list_messages(&thread.id, 10_000, 0)?;
        exported.push(ThreadExport { thread, messages });
    }
    let payload = HistoryExport {
        project: project.name,
        exported_at: chrono::Utc::now().to_rfc3339(),
        threads: exported,
    };
    serde_json::to_string_pretty(&payload)
        .map_err(|e| IhError::Internal(format!("export JSON : {e}")))
}

// ------------------------------------------------------------- automatisations

#[tauri::command]
pub fn workflows_list(state: State<'_, AppState>, project_id: String) -> IhResult<Vec<Workflow>> {
    let id = ProjectId::parse(project_id)?;
    state.store.list_workflows(&id)
}

#[tauri::command]
pub fn workflow_upsert(
    state: State<'_, AppState>,
    project_id: String,
    name: String,
    interval_minutes: u32,
    steps: serde_json::Value,
) -> IhResult<Workflow> {
    let project = ProjectId::parse(project_id)?;
    validate_workflow(&name, interval_minutes, &steps)?;
    let workflow = Workflow {
        id: format!("wf-{}", uuid::Uuid::new_v4()),
        project_id: project,
        name,
        interval_minutes,
        enabled: true,
        steps,
        last_run_at: None,
        created_at: chrono::Utc::now(),
    };
    state.store.upsert_workflow(&workflow)?;
    Ok(workflow)
}

#[tauri::command]
pub fn workflow_update(
    state: State<'_, AppState>,
    id: String,
    name: String,
    interval_minutes: u32,
    steps: serde_json::Value,
) -> IhResult<Workflow> {
    let mut workflow = find_workflow(&state, &id)?;
    validate_workflow(&name, interval_minutes, &steps)?;
    workflow.name = name;
    workflow.interval_minutes = interval_minutes;
    workflow.steps = steps;
    state.store.upsert_workflow(&workflow)?;
    Ok(workflow)
}

#[tauri::command]
pub fn workflow_set_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> IhResult<Workflow> {
    let mut workflow = find_workflow(&state, &id)?;
    workflow.enabled = enabled;
    state.store.upsert_workflow(&workflow)?;
    Ok(workflow)
}

#[tauri::command]
pub fn workflow_delete(state: State<'_, AppState>, id: String) -> IhResult<()> {
    state.store.delete_workflow(&id)
}

#[tauri::command]
pub fn workflow_run_now(state: State<'_, AppState>, id: String) -> IhResult<WorkflowRun> {
    let workflow = find_workflow(&state, &id)?;
    execute_workflow(&state, &workflow)
}

/// Valide les champs communs à la création et à la modification.
fn validate_workflow(name: &str, interval_minutes: u32, steps: &serde_json::Value) -> IhResult<()> {
    if name.trim().is_empty() {
        return Err(IhError::Invalid("nom de workflow vide".into()));
    }
    if interval_minutes < 5 {
        return Err(IhError::Invalid(
            "intervalle minimal d'automatisation : 5 minutes".into(),
        ));
    }
    let steps_array = steps
        .as_array()
        .ok_or_else(|| IhError::Invalid("les étapes doivent être un tableau".into()))?;
    if steps_array.is_empty() {
        return Err(IhError::Invalid("workflow sans étape".into()));
    }
    Ok(())
}

/// Retrouve un workflow par identifiant, tous projets confondus.
pub fn find_workflow(state: &AppState, id: &str) -> IhResult<Workflow> {
    state
        .store
        .get_workflow(id)?
        .ok_or_else(|| IhError::NotFound(format!("workflow {id}")))
}

/// Exécute réellement les étapes d'un workflow (agents supervisés, séquentiel,
/// arrêt à la première étape non terminée) puis enregistre le run.
///
/// Partagé entre l'exécution manuelle (`workflow_run_now`) et le planificateur
/// d'arrière-plan (`crate::automation`).
pub fn execute_workflow(state: &AppState, workflow: &Workflow) -> IhResult<WorkflowRun> {
    let config = state.config_snapshot()?;
    let root = project_path(state, workflow.project_id.as_str())?;

    let steps = workflow.steps.as_array().cloned().unwrap_or_default();
    let mut detail = String::new();
    let mut failure: Option<String> = None;

    for (index, step) in steps.iter().enumerate() {
        let agent_id = step
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IhError::Invalid(format!("étape {index} sans agent_id")))?;
        let prompt = step
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IhError::Invalid(format!("étape {index} sans prompt")))?;
        let agent_config = config
            .agent(agent_id)
            .ok_or_else(|| IhError::NotFound(format!("agent {agent_id}")))?
            .clone();
        let adapter = ih_agents::AgentAdapter::from_config(&agent_config);
        let session = state.supervisor.start(ih_agents::StartRequest {
            agent: adapter,
            project_dir: root.clone(),
            prompt: prompt.to_string(),
            session_id: None,
        });
        match session {
            Ok(session_id) => {
                let timeout = Duration::from_secs(agent_config.timeout_secs + 30);
                let state_after = state.supervisor.wait(&session_id, timeout)?;
                detail.push_str(&format!(
                    "étape {} ({agent_id}) : {:?}\n",
                    index + 1,
                    state_after
                ));
                if !matches!(state_after, ih_core::events::AgentState::Completed) {
                    failure = Some(format!("étape {} terminée en {:?}", index + 1, state_after));
                    break;
                }
            }
            Err(e) => {
                failure = Some(format!("étape {} : {e}", index + 1));
                break;
            }
        }
    }

    let run = state.store.record_workflow_run(&ih_store::NewWorkflowRun {
        workflow_id: workflow.id.clone(),
        status: if failure.is_some() {
            "failed".into()
        } else {
            "completed".into()
        },
        detail: Some(failure.clone().unwrap_or(detail)),
    })?;
    Ok(run)
}

#[tauri::command]
pub fn workflow_runs(
    state: State<'_, AppState>,
    id: String,
    limit: Option<u32>,
) -> IhResult<Vec<WorkflowRun>> {
    state.store.list_workflow_runs(&id, limit.unwrap_or(20))
}

// -------------------------------------------------------------- mises à jour

#[tauri::command]
pub fn app_version() -> String {
    ih_core::APP_VERSION.to_string()
}

#[tauri::command]
pub async fn update_check(state: State<'_, AppState>) -> IhResult<UpdateStatus> {
    let config = state.config_snapshot()?;
    let (Some(endpoint), Some(key)) = (
        config.updates.endpoint.clone(),
        config.updates.public_key_hex.clone(),
    ) else {
        return Ok(UpdateStatus {
            state: "not_configured".into(),
            error: Some(
                "endpoint de mise à jour ou clé publique absent de la configuration".into(),
            ),
            ..Default::default()
        });
    };

    let client = ih_update::UpdateClient::new()?;
    let signature_url = format!("{endpoint}.sig");
    let status = match client.fetch_verified(&endpoint, &signature_url, &key).await {
        Ok((manifest, _bytes)) => {
            let newer = ih_update::is_newer(ih_core::APP_VERSION, &manifest.version)?;
            UpdateStatus {
                state: if newer { "available" } else { "up_to_date" }.into(),
                version: Some(manifest.version),
                notes: Some(manifest.notes),
                progress: 0,
                error: None,
            }
        }
        Err(e) => UpdateStatus {
            state: "error".into(),
            version: None,
            notes: None,
            progress: 0,
            error: Some(e.to_string()),
        },
    };
    if let Ok(mut guard) = state.update_status.lock() {
        *guard = status.clone();
    }
    Ok(status)
}

#[tauri::command]
pub fn update_status(state: State<'_, AppState>) -> UpdateStatus {
    state
        .update_status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub fn update_plan(state: State<'_, AppState>) -> IhResult<String> {
    let staged = state
        .staged_update
        .lock()
        .map_err(|_| IhError::Internal("verrou de mise à jour".into()))?;
    let Some(update) = staged.as_ref() else {
        return Err(IhError::NotFound(
            "aucune mise à jour téléchargée et vérifiée".into(),
        ));
    };
    let current =
        std::env::current_exe().map_err(|e| IhError::Io(std::io::Error::other(e.to_string())))?;
    let plan = ih_update::plan_install(update, &current)?;
    Ok(format!("{plan:?}"))
}

#[tauri::command]
pub fn update_apply(state: State<'_, AppState>, approval_id: String) -> IhResult<String> {
    state.approvals.consume(&approval_id, "install")?;
    let staged = state
        .staged_update
        .lock()
        .map_err(|_| IhError::Internal("verrou de mise à jour".into()))?;
    let Some(update) = staged.as_ref() else {
        return Err(IhError::NotFound(
            "aucune mise à jour téléchargée et vérifiée".into(),
        ));
    };
    let current =
        std::env::current_exe().map_err(|e| IhError::Io(std::io::Error::other(e.to_string())))?;
    let plan = ih_update::plan_install(update, &current)?;
    match plan {
        ih_update::InstallPlan::ReplaceAppImage { .. } => {
            let backup = ih_update::apply_appimage(&plan, &state.paths.backups_dir())?;
            Ok(format!(
                "AppImage remplacée ; sauvegarde dans {}",
                backup.display()
            ))
        }
        ih_update::InstallPlan::RunPackageManager { command, args } => Ok(format!(
            "installation à effectuer manuellement : {command} {}",
            args.join(" ")
        )),
    }
}

#[tauri::command]
pub fn update_request_approval(state: State<'_, AppState>) -> IhResult<String> {
    let request = state.approvals.ask(
        "install",
        "Installer la mise à jour IH",
        "Le paquet a été vérifié (signature Ed25519 + SHA-256).",
        RiskLevel::High,
    )?;
    Ok(request.id.into_string())
}

/// Télécharge et vérifie l'artefact de mise à jour, puis le garde en attente.
#[tauri::command]
pub async fn update_download(state: State<'_, AppState>) -> IhResult<UpdateStatus> {
    let config = state.config_snapshot()?;
    let (Some(endpoint), Some(key)) = (
        config.updates.endpoint.clone(),
        config.updates.public_key_hex.clone(),
    ) else {
        return Err(IhError::Config(
            "endpoint ou clé publique de mise à jour absent".into(),
        ));
    };
    let client = ih_update::UpdateClient::new()?;
    let signature_url = format!("{endpoint}.sig");
    let (manifest, _bytes) = client
        .fetch_verified(&endpoint, &signature_url, &key)
        .await?;
    let kind = if std::env::var_os("APPIMAGE").is_some() {
        ih_update::ArtifactKind::AppImage
    } else {
        ih_update::ArtifactKind::Deb
    };
    let staged =
        ih_update::StagedUpdate::prepare(&client, &manifest, kind, &state.paths.staging_dir())
            .await?;
    let version = staged.manifest.version.clone();
    *state
        .staged_update
        .lock()
        .map_err(|_| IhError::Internal("verrou de mise à jour".into()))? = Some(staged);
    let status = UpdateStatus {
        state: "verified".into(),
        version: Some(version),
        notes: None,
        progress: 100,
        error: None,
    };
    if let Ok(mut guard) = state.update_status.lock() {
        *guard = status.clone();
    }
    Ok(status)
}
