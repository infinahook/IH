//! Commandes du mode Agent : adaptateurs, sessions isolées, approbations.

use super::project_path;
use crate::state::{AppState, PendingDiff};
use ih_agent_mode::{
    action_fingerprint, collect_changes, create_sandbox, detect_adapters, remove_sandbox,
    AdapterReport,
};
use ih_agents::sandbox::{self, SandboxSummary};
use ih_agents::{AgentAdapter, StartRequest};
use ih_core::config::SecurityLevel;
use ih_core::events::AgentState;
use ih_core::{IhError, IhResult, ProjectId, SessionId};
use ih_security::approval::{ApprovalDecision, ApprovalRequest};
use ih_security::risk::RiskLevel;
use ih_store::{AgentSessionDetail, NewSession};
use serde::Serialize;
use tauri::{Emitter, State};

/// Catégorie d'approbation pour le lancement d'une session d'agent.
const AGENT_CATEGORY: &str = "agent_run";

#[tauri::command]
pub fn agent_adapters(state: State<'_, AppState>) -> IhResult<Vec<AdapterReport>> {
    let config = state.config_snapshot()?;
    Ok(detect_adapters(&config.agents))
}

#[derive(Debug, Serialize)]
pub struct AgentApprovalView {
    pub approval_id: String,
    pub agent_id: String,
    pub label: String,
    pub command: String,
    pub workdir: String,
    pub risk: RiskLevel,
    pub fingerprint: String,
    pub detail: String,
    /// Niveau de sécurité demandé (`isolated`, `isolated_network`, `none`).
    pub security: SecurityLevel,
    pub sandbox: SandboxSummary,
}

/// Prépare une demande d'approbation pour un lancement de session.
///
/// L'empreinte (agent + commande + répertoire + tâche) est inscrite dans le
/// détail : toute modification ultérieure produit une empreinte différente et
/// le lancement est refusé.
#[tauri::command]
pub fn agent_session_request_approval(
    state: State<'_, AppState>,
    project_id: String,
    agent_id: String,
    task: String,
    // Confirmation explicite exigée pour le niveau non isolé.
    unsandboxed_ack: Option<bool>,
) -> IhResult<AgentApprovalView> {
    if task.trim().is_empty() {
        return Err(IhError::Invalid("tâche vide".into()));
    }
    let root = project_path(&state, &project_id)?;
    let config = state.config_snapshot()?;
    let agent_config = config
        .agent(&agent_id)
        .ok_or_else(|| IhError::NotFound(format!("agent {agent_id}")))?;
    let adapter = AgentAdapter::from_config(agent_config);
    let level = adapter.security;
    let sandbox = sandbox::summary(level);
    if level == SecurityLevel::None && unsandboxed_ack != Some(true) {
        return Err(IhError::Denied(
            "niveau non isolé : confirmez explicitement l'exécution sans sandbox avant de demander l'approbation"
                .into(),
        ));
    }
    if level.sandboxed() && !sandbox.engine_ready {
        return Err(IhError::Agent(format!(
            "sandbox indisponible : le lancement isolé est refusé. {}",
            sandbox
                .error
                .clone()
                .unwrap_or_else(|| "sandbox non fonctionnelle".into())
        )));
    }
    if adapter.detection().is_none() {
        return Err(IhError::Agent(format!(
            "programme introuvable dans le PATH : {}",
            adapter.program
        )));
    }
    let (_program, args) = adapter.build_command(&task);
    let command = format!("{} {}", adapter.program, args.join(" "));
    // Le worktree n'existe pas encore : on empreinte la racine du projet comme
    // répertoire de session (le worktree est créé sous ce dernier).
    // Le niveau de sécurité fait partie de l'empreinte : changer le niveau
    // invalide l'approbation existante.
    let fingerprint = action_fingerprint(
        &agent_id,
        &format!("{command} [securite:{}]", level.as_str()),
        &root,
        &task,
    );

    let mut detail = format!(
        "Commande : {command}\nRépertoire isolé créé sous : {}\nEmpreinte : {fingerprint}\n\
         Niveau de sécurité : {} (réseau : {})\n\
         L'agent s'exécute dans une copie (worktree git) : le projet n'est pas modifié \
         pendant la session. Les changements seront présentés en diff avant application.",
        root.display(),
        level.as_str(),
        if level.network_enabled() {
            "activé"
        } else {
            "désactivé"
        }
    );
    if level.sandboxed() {
        detail.push_str(&format!(
            "\nSandbox : {} ({}) — dossier personnel privé, /usr et /etc en lecture seule, \
             /tmp privé, écriture limitée au worktree.",
            sandbox
                .engine
                .clone()
                .unwrap_or_else(|| "bubblewrap".into()),
            if sandbox.engine_ready {
                "opérationnelle"
            } else {
                "indisponible"
            }
        ));
    }
    for warning in &sandbox.warnings {
        detail.push_str(&format!("\n⚠ {warning}"));
    }
    // Le worktree est une isolation Git, pas une sandbox système : un drapeau
    // d'autonomie (ex. `--auto`) permet à l'agent d'écrire, d'exécuter des
    // commandes ou de lire des fichiers hors du worktree avec les droits de
    // l'utilisateur. Ce risque doit être explicite au moment d'approuver.
    let autonomy = adapter.autonomy_flags();
    if !autonomy.is_empty() {
        detail.push_str(&format!(
            "\n\n⚠ Mode autonome ({}) : l'agent peut écrire, supprimer, exécuter des commandes \
             et accéder au réseau ou à des fichiers HORS du worktree, avec les droits de votre \
             compte utilisateur. L'isolation par worktree ne couvre que les changements Git du \
             projet ; les actions externes ne sont pas récupérées par le diff.",
            autonomy.join(" ")
        ));
    }
    let request = state.approvals.ask(
        AGENT_CATEGORY,
        &format!("Lancer {} sur : {}", adapter.label, truncate(&task, 120)),
        &detail,
        RiskLevel::High,
    )?;
    Ok(AgentApprovalView {
        approval_id: request.id.into_string(),
        agent_id,
        label: adapter.label,
        command,
        workdir: root.display().to_string(),
        risk: RiskLevel::High,
        fingerprint,
        detail,
        security: level,
        sandbox,
    })
}

#[derive(Debug, Serialize)]
pub struct AgentSessionStarted {
    pub session_id: String,
    pub workdir: String,
    pub agent_id: String,
}

/// Démarre une session d'agent après consommation d'une approbation ponctuelle.
#[tauri::command]
pub fn agent_session_start(
    state: State<'_, AppState>,
    project_id: String,
    thread_id: Option<String>,
    agent_id: String,
    task: String,
    approval_id: String,
    timeout_secs: Option<u64>,
) -> IhResult<AgentSessionStarted> {
    if task.trim().is_empty() {
        return Err(IhError::Invalid("tâche vide".into()));
    }
    let project = ProjectId::parse(project_id)?;
    let root = project_path(&state, project.as_str())?;
    let config = state.config_snapshot()?;
    let agent_config = config
        .agent(&agent_id)
        .ok_or_else(|| IhError::NotFound(format!("agent {agent_id}")))?
        .clone();
    let adapter = AgentAdapter::from_config(&agent_config);
    if adapter.detection().is_none() {
        return Err(IhError::Agent(format!(
            "programme introuvable dans le PATH : {}",
            adapter.program
        )));
    }
    if state.supervisor.active_count() >= config.resources.max_concurrent_agents as usize {
        return Err(IhError::Limit(format!(
            "maximum de {} sessions simultanées atteint",
            config.resources.max_concurrent_agents
        )));
    }

    // 1. Vérifie l'empreinte AVANT de consommer l'approbation.
    let level = adapter.security;
    let (_program, args) = adapter.build_command(&task);
    let command = format!("{} {}", adapter.program, args.join(" "));
    // Le niveau de sécurité fait partie de l'empreinte : changer le niveau
    // invalide l'approbation existante.
    let fingerprint = action_fingerprint(
        &agent_id,
        &format!("{command} [securite:{}]", level.as_str()),
        &root,
        &task,
    );
    let pending = state
        .approvals
        .get(&approval_id)?
        .ok_or_else(|| IhError::NotFound(format!("approbation {approval_id}")))?;
    if !pending.detail.contains(&fingerprint) {
        return Err(IhError::Denied(
            "l'approbation ne correspond pas à cette commande ou à cette tâche".into(),
        ));
    }
    state.approvals.consume(&approval_id, AGENT_CATEGORY)?;

    // 2. Session persistée puis sandbox isolé.
    let session = state.store.create_session(&NewSession {
        project_id: project.clone(),
        agent_id: agent_id.clone(),
        thread_id: thread_id.map(ih_core::ThreadId::parse).transpose()?,
        prompt: task.clone(),
    })?;
    let base_dir = state.paths.data_dir().join("worktrees");
    let sandbox = create_sandbox(&root, &base_dir, session.id.as_str())?;
    let workdir = sandbox.workdir().to_path_buf();
    state.store.update_session_meta(
        &session.id,
        Some(&agent_id),
        Some(&task),
        Some(&workdir.display().to_string()),
    )?;
    {
        let mut set = state
            .agent_sessions
            .lock()
            .map_err(|_| IhError::Internal("registre d'agents verrouillé".into()))?;
        set.insert(session.id.to_string());
    }

    // 3. Lancement dans le worktree.
    let session_id = session.id.clone();
    let supervisor_id = state.supervisor.start(StartRequest {
        agent: adapter.clone(),
        project_dir: workdir.clone(),
        prompt: task.clone(),
        session_id: Some(session_id.clone()),
        // L'approbation consommée couvre la commande ET le niveau de sécurité :
        // un niveau non isolé n'est lancé qu'après cette approbation explicite.
        confirm_unsandboxed: adapter.security == ih_core::config::SecurityLevel::None,
    })?;
    if supervisor_id != session_id.to_string() {
        return Err(IhError::Internal(
            "l'identifiant de session du superviseur diffère de celui de la base".into(),
        ));
    }
    state
        .store
        .update_session_state(&session_id, AgentState::Running, None)?;
    let _ = state.app_handle.emit(
        "ih://agent-event",
        serde_json::json!({
            "session_id": session.id.to_string(),
            "seq": -1,
            "kind": "started",
            "payload": {
                "agent_id": agent_id,
                "workdir": workdir.display().to_string(),
                "command": command,
                "task": task,
            },
        }),
    );

    // 4. Fin de session : changements, résumé, nettoyage, diffs en attente.
    let store = state.store.clone();
    let app_handle = state.app_handle.clone();
    let pending_diffs = state.pending_diffs.clone();
    let agent_sessions = state.agent_sessions.clone();
    let supervisor = state.supervisor.clone();
    let timeout = timeout_secs.unwrap_or(agent_config.timeout_secs).max(5);
    let session_for_thread = session.id.clone();
    let project_for_thread = project.clone();
    let root_for_thread = root.clone();
    let sandbox_for_thread = sandbox.clone();

    std::thread::spawn(move || {
        let final_state = supervisor
            .wait(
                session_for_thread.as_str(),
                std::time::Duration::from_secs(timeout.saturating_add(30)),
            )
            .unwrap_or(AgentState::Failed);

        let mut files: Vec<String> = Vec::new();
        let mut prepared = 0usize;
        let mut error_note: Option<String> = None;
        match collect_changes(&sandbox_for_thread, ih_agent_mode::MAX_CHANGE_BYTES) {
            Ok(changes) => {
                for change in &changes {
                    if change.binary || change.too_large {
                        continue;
                    }
                    files.push(change.path.clone());
                    let diff_id = format!("diff-{}", uuid::Uuid::new_v4());
                    let before_sha = project_sha(&root_for_thread, &change.path);
                    let entry = PendingDiff {
                        project_id: project_for_thread.to_string(),
                        path: change.path.clone(),
                        before_sha256: before_sha,
                        content: change.after.clone().unwrap_or_default(),
                        delete: change.after.is_none(),
                        origin: format!("agent:{session_for_thread}"),
                        created_at: chrono::Utc::now(),
                    };
                    if let Ok(mut guard) = pending_diffs.lock() {
                        guard.insert(diff_id, entry);
                        prepared += 1;
                    }
                }
            }
            Err(error) => error_note = Some(error.to_string()),
        }

        if let Ok(mut set) = agent_sessions.lock() {
            set.remove(session_for_thread.as_str());
        }
        let summary = match (final_state, prepared) {
            (AgentState::Cancelled, _) => "session annulée".to_string(),
            (state, _) if files.is_empty() => {
                format!("terminé ({state:?}) — aucun fichier modifié")
            }
            (_state, prepared_count) => {
                format!("{prepared_count} fichier(s) modifié(s), en attente de validation")
            }
        };
        let _ = store.complete_session(
            &session_for_thread,
            final_state,
            None,
            Some(&summary),
            &files,
        );
        let _ = app_handle.emit(
            "ih://agent-event",
            serde_json::json!({
                "session_id": session_for_thread.to_string(),
                "seq": -1,
                "kind": match final_state {
                    AgentState::Completed => "completed",
                    AgentState::Cancelled => "cancelled",
                    _ => "error",
                },
                "payload": {
                    "summary": summary,
                    "files": files,
                    "changes_prepared": prepared,
                    "error": error_note,
                },
            }),
        );
        let _ = remove_sandbox(&sandbox_for_thread);
    });

    Ok(AgentSessionStarted {
        session_id: session.id.into_string(),
        workdir: workdir.display().to_string(),
        agent_id,
    })
}

#[tauri::command]
pub fn agent_session_cancel(state: State<'_, AppState>, session_id: String) -> IhResult<()> {
    state.supervisor.cancel(&session_id)?;
    let id = SessionId::parse(session_id)?;
    state
        .store
        .update_session_state(&id, AgentState::Cancelled, None)?;
    Ok(())
}

#[tauri::command]
pub fn agent_session_list(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<u32>,
) -> IhResult<Vec<AgentSessionDetail>> {
    let project = ProjectId::parse(project_id)?;
    let sessions = state.store.list_sessions(&project, limit.unwrap_or(30))?;
    let mut details = Vec::new();
    for session in sessions {
        if let Some(detail) = state.store.get_session_detail(&session.id)? {
            details.push(detail);
        }
    }
    Ok(details)
}

#[tauri::command]
pub fn agent_session_detail(
    state: State<'_, AppState>,
    session_id: String,
) -> IhResult<AgentSessionDetail> {
    let id = SessionId::parse(session_id)?;
    state
        .store
        .get_session_detail(&id)?
        .ok_or_else(|| IhError::NotFound(format!("session {id}")))
}

#[tauri::command]
pub fn agent_session_events(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<u32>,
) -> IhResult<Vec<ih_store::AgentEventRow>> {
    let id = SessionId::parse(session_id)?;
    state.store.list_agent_events(&id, limit.unwrap_or(500))
}

/// Annule une approbation en attente (bouton « Refuser »).
#[tauri::command]
pub fn agent_session_refuse(
    state: State<'_, AppState>,
    approval_id: String,
) -> IhResult<ApprovalRequest> {
    state
        .approvals
        .resolve(&approval_id, ApprovalDecision::Deny)
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

/// Empreinte SHA-256 du fichier dans le projet (None si absent ou illisible).
fn project_sha(root: &std::path::Path, path: &str) -> Option<String> {
    ih_workspace::fs::ProjectFs::new(root)
        .ok()?
        .metadata(path)
        .ok()
        .map(|meta| meta.sha256)
}

#[cfg(test)]
mod real_agent_detection_tests {

    /// Vérifie la charge utile réellement renvoyée par `agent_adapters` sur une
    /// configuration par défaut, avec les CLI installés (Kilo, DSH CLI, Goose).
    ///
    /// `#[ignore]` : la CI n'a pas ces binaires. Exécution explicite :
    /// `IH_REAL_AGENTS=1 cargo test -p ih-desktop --lib -- --ignored`.
    #[test]
    #[ignore = "agents CLI réels requis : exécuter avec --ignored (IH_REAL_AGENTS=1)"]
    fn les_trois_nouveaux_agents_sont_detectes_avec_version() {
        let config = ih_core::config::AppConfig::default();
        let reports = ih_agent_mode::detect_adapters(&config.agents);
        for expected in ["kilo", "dsh-cli", "goose"] {
            let report = reports
                .iter()
                .find(|report| report.id == expected)
                .unwrap_or_else(|| {
                    panic!("agent {expected} absent de la configuration par défaut")
                });
            println!(
                "[{}] path={:?} version={:?} functional={} diagnostic={}",
                expected, report.path, report.version, report.functional, report.diagnostic
            );
            assert!(report.detected, "{expected} non détecté");
            assert!(report.functional, "{expected} non fonctionnel");
            assert!(report.version.is_some(), "{expected} sans version");
        }
    }
}
