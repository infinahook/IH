//! Planificateur d'arrière-plan des automatisations.
//!
//! Toutes les 30 secondes, les workflows activés dont l'intervalle est écoulé
//! (`Store::due_workflows`) sont exécutés hors du thread principal. Chaque
//! exécution passe par le même surveillant d'agents et le même enregistrement
//! en base que l'exécution manuelle (`workflow_run_now`), puis émet
//! `ih://workflow-run` (phase `started` puis `finished`) pour que l'interface
//! affiche l'exécution planifiée sans interrogation périodique.
//!
//! Un workflow déjà en cours d'exécution n'est jamais relancé : l'ensemble
//! `in_flight` couvre la fenêtre entre le balayage et la fin du thread.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ih_store::{NewWorkflowRun, Workflow, WorkflowRun};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::ops::execute_workflow;
use crate::state::AppState;

/// Intervalle de balayage : au pire 30 s de décalage sur l'intervalle minimal
/// de 5 minutes accepté à la création d'un workflow.
const SWEEP: Duration = Duration::from_secs(30);

/// Démarre le planificateur : premier balayage immédiat, puis toutes les 30 s.
pub fn start(app: AppHandle) {
    let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    tauri::async_runtime::spawn(async move {
        loop {
            sweep(&app, &in_flight);
            tokio::time::sleep(SWEEP).await;
        }
    });
}

/// Un tour de planification : lit les workflows dus et lance ceux qui ne sont
/// pas déjà en cours.
fn sweep(app: &AppHandle, in_flight: &Arc<Mutex<HashSet<String>>>) {
    let due = {
        let state = app.state::<AppState>();
        match state.store.due_workflows(chrono::Utc::now()) {
            Ok(list) => list,
            Err(err) => {
                tracing::warn!(error = %err, "lecture des workflows planifiés impossible");
                return;
            }
        }
    };

    for workflow in due {
        let workflow_id = workflow.id.clone();
        let mut guard = match in_flight.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !guard.insert(workflow_id.clone()) {
            // Exécution toujours en cours : on ne double pas le run.
            continue;
        }
        drop(guard);

        let app = app.clone();
        let in_flight = Arc::clone(in_flight);
        std::thread::spawn(move || {
            run_one(&app, &workflow);
            let mut guard = match in_flight.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.remove(&workflow_id);
        });
    }
}

/// Exécute un workflow et diffuse son état à l'interface.
fn run_one(app: &AppHandle, workflow: &Workflow) {
    emit(app, workflow, "started", None, None);
    let state = app.state::<AppState>();
    match execute_workflow(state.inner(), workflow) {
        Ok(run) => emit(app, workflow, "finished", Some(run), None),
        Err(err) => {
            tracing::warn!(
                workflow = workflow.id.as_str(),
                error = %err,
                "exécution planifiée en échec avant toute étape"
            );
            // Consigne tout de même l'échec pour que l'intervalle reparte :
            // sans cela un workflow invalide serait relancé à chaque balayage.
            let _ = state.store.record_workflow_run(&NewWorkflowRun {
                workflow_id: workflow.id.clone(),
                status: "failed".into(),
                detail: Some(err.to_string()),
            });
            emit(app, workflow, "finished", None, Some(err.to_string()));
        }
    }
}

/// Charge envoyée à l'interface sur `ih://workflow-run`.
#[derive(Debug, Clone, Serialize)]
struct WorkflowRunEvent {
    /// `started` ou `finished`.
    phase: String,
    workflow_id: String,
    project_id: String,
    name: String,
    /// Présent en phase `finished` quand l'exécution s'est déroulée.
    run: Option<WorkflowRun>,
    /// Présent en phase `finished` quand l'échec précède toute étape.
    error: Option<String>,
}

fn emit(
    app: &AppHandle,
    workflow: &Workflow,
    phase: &str,
    run: Option<WorkflowRun>,
    error: Option<String>,
) {
    let payload = WorkflowRunEvent {
        phase: phase.to_string(),
        workflow_id: workflow.id.clone(),
        project_id: workflow.project_id.as_str().to_string(),
        name: workflow.name.clone(),
        run,
        error,
    };
    let _ = app.emit("ih://workflow-run", payload);
}
