//! Tests d'intégration du store, exécutés avec `cargo test -p ih-store`.
//!
//! Chaque test travaille sur une base isolée (fichier temporaire ou mémoire) et
//! ne laisse aucune donnée derrière lui.

use std::path::PathBuf;
use std::sync::{Arc, Barrier};

use chrono::{Duration, Utc};
use ih_core::events::{AgentState, TaskState};
use ih_core::{IhError, MessageId, ProjectId, SessionId, TaskId, ThreadId};
use serde_json::json;

use super::*;

/// Répertoire temporaire supprimé à la fin du test.
struct TempDb {
    dir: PathBuf,
}

impl TempDb {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("ih-store-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("création du répertoire temporaire");
        Self { dir }
    }

    /// Chemin dans un sous-dossier volontairement absent : `Store::open` doit le
    /// créer.
    fn path(&self) -> PathBuf {
        self.dir.join("nested").join("ih.db")
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn sample_project(name: &str) -> Project {
    let now = Utc::now();
    Project {
        id: ProjectId::new(),
        path: format!("/tmp/ih-store-projects/{name}-{}", uuid::Uuid::new_v4()),
        name: name.to_owned(),
        created_at: now,
        last_opened_at: now,
        trusted: true,
    }
}

fn seed_project(store: &Store, name: &str) -> Project {
    let project = sample_project(name);
    store
        .upsert_project(&project)
        .expect("enregistrement du projet");
    project
}

fn new_message(thread_id: &ThreadId, role: &str, content: impl Into<String>) -> NewMessage {
    NewMessage {
        thread_id: thread_id.clone(),
        role: role.to_owned(),
        content: content.into(),
        metadata: None,
    }
}

fn new_workflow(project_id: &ProjectId, name: &str) -> Workflow {
    Workflow {
        id: format!("wf-{}", uuid::Uuid::new_v4()),
        project_id: project_id.clone(),
        name: name.to_owned(),
        interval_minutes: 1_440,
        enabled: true,
        steps: json!([{ "kind": "review", "prompt": "Relis les changements" }]),
        last_run_at: None,
        created_at: Utc::now(),
    }
}

fn new_extension(
    id: &str,
    kind: &str,
    version: &str,
    at: chrono::DateTime<Utc>,
) -> InstalledExtension {
    let name = if kind == "skill" { "Lint" } else { "mcp-fs" };
    InstalledExtension {
        id: id.to_owned(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        source: "local".to_owned(),
        manifest: json!({ "entry": "index.js", "permissions": ["fs:read"] }),
        sha256: None,
        enabled: true,
        installed_at: at,
        updated_at: at,
    }
}

// -------------------------------------------------------------------------
// 1. Ouverture, version de schéma, réouverture idempotente
// -------------------------------------------------------------------------

#[test]
fn ouverture_version_et_reouverture_idempotente() {
    let db = TempDb::new();
    let path = db.path();
    assert!(
        !path.exists(),
        "le sous-dossier ne doit pas exister avant open"
    );

    let project = sample_project("alpha");
    {
        let store = Store::open(&path).expect("ouverture initiale");
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.db_path().as_deref(), Some(path.as_path()));
        store.set_setting("locale", "fr").unwrap();
        store.upsert_project(&project).unwrap();
    }
    assert!(path.exists(), "le fichier SQLite doit être créé");

    {
        let store = Store::open(&path).expect("réouverture");
        assert_eq!(
            store.schema_version().unwrap(),
            SCHEMA_VERSION,
            "la réouverture ne doit pas rejouer les migrations"
        );
        assert_eq!(
            store.get_setting("locale").unwrap().as_deref(),
            Some("fr"),
            "les données doivent survivre à la réouverture"
        );
        assert_eq!(store.list_projects().unwrap().len(), 1);
    }
}

#[test]
fn ouverture_en_memoire_sans_chemin() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(store.db_path(), None);
}

#[test]
fn migration_v1_vers_courante_preserve_les_donnees() {
    let db = TempDb::new();
    let path = db.path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }

    // Simule une base créée par la v1, avant l'ajout des index de la v2.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(crate::migrate::V1).unwrap();
        conn.pragma_update(None, "user_version", 1_i64).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params!["legacy", "v1", "2024-01-01T00:00:00+00:00"],
        )
        .unwrap();
    }

    let store = Store::open(&path).unwrap();
    // Les migrations v2 et v3 s'enchaînent sur une base v1.
    assert_eq!(store.schema_version().unwrap(), crate::SCHEMA_VERSION);
    assert_eq!(store.get_setting("legacy").unwrap().as_deref(), Some("v1"));
}

#[test]
fn sessions_agent_et_evenements_persistes() {
    let store = Store::open_in_memory().unwrap();
    let project = crate::Project {
        id: ih_core::ProjectId::new(),
        path: "/tmp/ih-agent-test".into(),
        name: "agent".into(),
        created_at: chrono::Utc::now(),
        last_opened_at: chrono::Utc::now(),
        trusted: true,
    };
    store.upsert_project(&project).unwrap();
    let session = store
        .create_session(&crate::NewSession {
            project_id: project.id.clone(),
            agent_id: "claude-code".into(),
            thread_id: None,
            prompt: "analyse le dépôt".into(),
        })
        .unwrap();

    store
        .update_session_meta(
            &session.id,
            Some("claude-code"),
            Some("analyse le dépôt"),
            Some("/tmp/wt-1"),
        )
        .unwrap();
    for seq in 0..3 {
        store
            .append_agent_event(&crate::NewAgentEvent {
                session_id: session.id.clone(),
                seq,
                kind: "output".into(),
                payload: serde_json::json!({ "line": format!("ligne {seq}") }),
            })
            .unwrap();
    }
    store
        .complete_session(
            &session.id,
            ih_core::events::AgentState::Completed,
            Some(0),
            Some("terminé"),
            &["src/main.rs".to_string()],
        )
        .unwrap();

    let detail = store.get_session_detail(&session.id).unwrap().unwrap();
    assert_eq!(detail.adapter.as_deref(), Some("claude-code"));
    assert_eq!(detail.workdir.as_deref(), Some("/tmp/wt-1"));
    assert_eq!(detail.summary.as_deref(), Some("terminé"));
    assert_eq!(detail.files, vec!["src/main.rs".to_string()]);
    assert_eq!(detail.session.state, ih_core::events::AgentState::Completed);

    let events = store.list_agent_events(&session.id, 10).unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].seq, 2);
    assert_eq!(events[0].payload["line"], "ligne 0");
}

// -------------------------------------------------------------------------
// 2. CRUD projet
// -------------------------------------------------------------------------

#[test]
fn crud_projet_upsert_et_suppression_coherente() {
    let db = TempDb::new();
    let store = Store::open(&db.path()).unwrap();

    let mut project = sample_project("alpha");
    store.upsert_project(&project).unwrap();
    assert_eq!(store.list_projects().unwrap().len(), 1);
    assert_eq!(
        store.get_project(&project.id).unwrap().as_ref(),
        Some(&project)
    );

    let touched_at = Utc::now() + Duration::seconds(30);
    store.touch_project(&project.id, touched_at).unwrap();
    assert_eq!(
        store
            .get_project(&project.id)
            .unwrap()
            .unwrap()
            .last_opened_at,
        touched_at
    );

    project.name = "alpha-renomme".to_owned();
    project.trusted = false;
    project.last_opened_at = touched_at;
    store.upsert_project(&project).unwrap();

    let projects = store.list_projects().unwrap();
    assert_eq!(projects.len(), 1, "l'upsert ne doit pas créer de doublon");
    assert_eq!(projects[0].name, "alpha-renomme");
    assert!(!projects[0].trusted);
    assert_eq!(projects[0].created_at, project.created_at);

    let other = seed_project(&store, "beta");

    // Données liées au projet supprimé : elles doivent toutes partir en cascade.
    let thread = store
        .create_thread(&project.id, "Conversation", None)
        .unwrap();
    store
        .append_message(&new_message(&thread.id, "user", "bonjour"))
        .unwrap();
    store
        .create_task(&NewTask {
            project_id: project.id.clone(),
            title: "Tâche".into(),
            description: String::new(),
            assignee: None,
            depends_on: Vec::new(),
            role: "worker".into(),
        })
        .unwrap();
    store
        .create_session(&NewSession {
            project_id: project.id.clone(),
            agent_id: "claude-code".into(),
            thread_id: Some(thread.id.clone()),
            prompt: "Analyse".into(),
        })
        .unwrap();
    store
        .add_memory(&NewMemoryEntry {
            project_id: Some(project.id.clone()),
            scope: "project".into(),
            key: None,
            content: "mémoire locale".into(),
            tags: vec!["local".into()],
            source: "test".into(),
        })
        .unwrap();
    store
        .add_memory(&NewMemoryEntry {
            project_id: None,
            scope: "global".into(),
            key: None,
            content: "mémoire globale".into(),
            tags: vec!["global".into()],
            source: "test".into(),
        })
        .unwrap();
    let workflow = new_workflow(&project.id, "veille");
    store.upsert_workflow(&workflow).unwrap();
    store
        .record_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id.clone(),
            status: "success".into(),
            detail: None,
        })
        .unwrap();

    store.remove_project(&project.id).unwrap();

    assert!(store.get_project(&project.id).unwrap().is_none());
    assert_eq!(store.list_projects().unwrap().len(), 1);
    assert!(store.get_thread(&thread.id).unwrap().is_none());
    assert!(store.list_messages(&thread.id, 10, 0).unwrap().is_empty());
    assert!(store.list_tasks(&project.id).unwrap().is_empty());
    assert!(store.list_sessions(&project.id, 10).unwrap().is_empty());
    assert!(store.list_memory(Some(&project.id)).unwrap().is_empty());
    assert!(store.list_workflows(&project.id).unwrap().is_empty());
    assert!(store
        .list_workflow_runs(&workflow.id, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.list_memory(None).unwrap().len(),
        1,
        "la mémoire globale ne dépend d'aucun projet et doit rester"
    );
    assert_eq!(store.get_project(&other.id).unwrap().unwrap().name, "beta");

    let error = store.remove_project(&project.id).unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 3. Conversations et messages
// -------------------------------------------------------------------------

#[test]
fn conversations_messages_pagination_et_cascade() {
    let store = Store::open_in_memory().unwrap();
    let project = seed_project(&store, "chat");
    let thread = store
        .create_thread(&project.id, "Brouillon", Some("claude-code"))
        .unwrap();
    assert_eq!(thread.agent_id.as_deref(), Some("claude-code"));
    assert_eq!(thread.created_at, thread.updated_at);

    let mut expected = Vec::new();
    let mut seen_ids = std::collections::HashSet::<MessageId>::new();
    for index in 0..10_u32 {
        let role = if index % 2 == 0 { "user" } else { "assistant" };
        let metadata = (index == 0).then(|| json!({ "tokens": 12 }));
        let message = store
            .append_message(&NewMessage {
                thread_id: thread.id.clone(),
                role: role.into(),
                content: format!("message-{index}"),
                metadata,
            })
            .unwrap();
        assert!(seen_ids.insert(message.id), "identifiants uniques");
        expected.push(format!("message-{index}"));
    }

    let all = store.list_messages(&thread.id, 100, 0).unwrap();
    assert_eq!(
        all.iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
        expected,
        "ordre chronologique d'insertion"
    );
    assert_eq!(all[0].metadata, Some(json!({ "tokens": 12 })));
    assert!(all[1].metadata.is_none());

    let page = store.list_messages(&thread.id, 3, 2).unwrap();
    assert_eq!(page.len(), 3);
    assert_eq!(page[0].content, "message-2");
    assert_eq!(page[1].content, "message-3");
    assert_eq!(page[2].content, "message-4");
    assert!(store.list_messages(&thread.id, 0, 0).unwrap().is_empty());

    store.rename_thread(&thread.id, "Renommée").unwrap();
    assert_eq!(
        store.get_thread(&thread.id).unwrap().unwrap().title,
        "Renommée"
    );

    store.delete_thread(&thread.id).unwrap();
    assert!(store.get_thread(&thread.id).unwrap().is_none());
    assert!(
        store.list_messages(&thread.id, 100, 0).unwrap().is_empty(),
        "ON DELETE CASCADE doit supprimer les messages"
    );
    assert!(
        store
            .append_message(&new_message(&thread.id, "user", "orphelin"))
            .is_err(),
        "la clé étrangère doit refuser un message orphelin"
    );
}

// -------------------------------------------------------------------------
// 4. Tâches
// -------------------------------------------------------------------------

#[test]
fn taches_etats_dependances_et_not_found() {
    let store = Store::open_in_memory().unwrap();
    let project = seed_project(&store, "orchestrateur");
    let depends_on = vec![TaskId::new(), TaskId::new()];

    let new_task = NewTask {
        project_id: project.id.clone(),
        title: "Implémenter le store".into(),
        description: "SQLite + migrations".into(),
        assignee: Some("claude-code".into()),
        depends_on: depends_on.clone(),
        role: "worker".into(),
    };
    let task = store.create_task(&new_task).unwrap();
    assert_eq!(task.state, TaskState::Pending);
    assert_eq!(task.depends_on, depends_on);

    let reloaded = store.get_task(&task.id).unwrap().unwrap();
    assert_eq!(
        reloaded.depends_on, depends_on,
        "depends_on relu à l'identique"
    );
    assert_eq!(reloaded.assignee.as_deref(), Some("claude-code"));
    assert_eq!(reloaded.role, "worker");

    store
        .update_task_state(&task.id, TaskState::Running)
        .unwrap();
    let running = store.get_task(&task.id).unwrap().unwrap();
    assert_eq!(running.state, TaskState::Running);
    assert!(running.updated_at >= task.updated_at);

    store
        .update_task_state(&task.id, TaskState::Completed)
        .unwrap();
    let tasks = store.list_tasks(&project.id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].state, TaskState::Completed);

    let missing = TaskId::new();
    let error = store
        .update_task_state(&missing, TaskState::Failed)
        .unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");

    let mut invalid = new_task;
    invalid.role = "chef".into();
    let error = store.create_task(&invalid).unwrap_err();
    assert!(matches!(error, IhError::Invalid(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 5. Mémoire
// -------------------------------------------------------------------------

#[test]
fn memoire_recherche_filtre_et_suppression() {
    let store = Store::open_in_memory().unwrap();
    let project = seed_project(&store, "memoire");

    let project_entry = store
        .add_memory(&NewMemoryEntry {
            project_id: Some(project.id.clone()),
            scope: "project".into(),
            key: Some("build-command".into()),
            content: "cargo test -p ih-store".into(),
            tags: vec!["rust".into(), "tests".into()],
            source: "agent".into(),
        })
        .unwrap();
    let global_entry = store
        .add_memory(&NewMemoryEntry {
            project_id: None,
            scope: "global".into(),
            key: None,
            content: "Toujours répondre en français".into(),
            tags: vec!["style".into()],
            source: "user".into(),
        })
        .unwrap();

    assert_eq!(store.list_memory(None).unwrap().len(), 2);
    let project_entries = store.list_memory(Some(&project.id)).unwrap();
    assert_eq!(project_entries.len(), 1);
    assert_eq!(project_entries[0].id, project_entry.id);
    assert_eq!(project_entries[0].tags, vec!["rust", "tests"]);

    let by_content = store
        .search_memory(Some(&project.id), "CARGO TEST", 10)
        .unwrap();
    assert_eq!(by_content.len(), 1, "LIKE est insensible à la casse ASCII");

    let by_key = store.search_memory(None, "BUILD", 10).unwrap();
    assert_eq!(by_key.len(), 1);
    assert_eq!(by_key[0].id, project_entry.id);

    let by_tag = store.search_memory(None, "tests", 10).unwrap();
    assert_eq!(by_tag.len(), 1);
    assert_eq!(by_tag[0].id, project_entry.id);

    assert!(
        store
            .search_memory(Some(&project.id), "style", 10)
            .unwrap()
            .is_empty(),
        "le filtre projet est strict : la mémoire globale n'est pas incluse"
    );
    assert_eq!(
        store.search_memory(None, "style", 10).unwrap()[0].id,
        global_entry.id
    );
    assert_eq!(store.search_memory(None, "français", 10).unwrap().len(), 1);

    store.delete_memory(&project_entry.id).unwrap();
    assert_eq!(store.list_memory(Some(&project.id)).unwrap().len(), 0);
    assert_eq!(store.list_memory(None).unwrap().len(), 1);
    let error = store.delete_memory(&project_entry.id).unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");

    let error = store
        .add_memory(&NewMemoryEntry {
            project_id: None,
            scope: "project".into(),
            key: None,
            content: "sans projet".into(),
            tags: Vec::new(),
            source: "test".into(),
        })
        .unwrap_err();
    assert!(matches!(error, IhError::Invalid(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 6. Extensions
// -------------------------------------------------------------------------

#[test]
fn extensions_upsert_version_et_filtre_kind() {
    let store = Store::open_in_memory().unwrap();
    let installed_at = Utc::now();
    let skill = new_extension("skill-lint", "skill", "1.0.0", installed_at);
    store.upsert_extension(&skill).unwrap();

    assert_eq!(store.list_extensions("skill").unwrap().len(), 1);
    assert!(store.list_extensions("plugin").unwrap().is_empty());

    let updated_at = installed_at + Duration::hours(1);
    let mut upgraded = skill.clone();
    upgraded.version = "1.1.0".into();
    upgraded.manifest = json!({ "entry": "dist/index.js" });
    upgraded.updated_at = updated_at;
    store.upsert_extension(&upgraded).unwrap();

    let skills = store.list_extensions("skill").unwrap();
    assert_eq!(skills.len(), 1, "upsert sur (id, kind) sans doublon");
    assert_eq!(skills[0].version, "1.1.0");
    assert_eq!(skills[0].manifest, json!({ "entry": "dist/index.js" }));
    assert_eq!(skills[0].installed_at, installed_at);
    assert_eq!(skills[0].updated_at, updated_at);

    let plugin = new_extension("plugin-fs", "plugin", "0.3.0", installed_at);
    store.upsert_extension(&plugin).unwrap();
    assert_eq!(store.list_extensions("plugin").unwrap().len(), 1);
    assert_eq!(store.list_extensions("skill").unwrap().len(), 1);

    store.remove_extension(&plugin.id, "plugin").unwrap();
    assert!(store.list_extensions("plugin").unwrap().is_empty());
    assert_eq!(store.list_extensions("skill").unwrap().len(), 1);

    let error = store.remove_extension(&plugin.id, "plugin").unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");

    let error = store.list_extensions("mcp").unwrap_err();
    assert!(matches!(error, IhError::Invalid(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 7. Workflows et exécutions
// -------------------------------------------------------------------------

#[test]
fn workflows_runs_et_mise_a_jour() {
    let store = Store::open_in_memory().unwrap();
    let project = seed_project(&store, "workflows");
    let workflow = new_workflow(&project.id, "veille");
    store.upsert_workflow(&workflow).unwrap();

    let workflows = store.list_workflows(&project.id).unwrap();
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].steps, workflow.steps);
    assert!(workflows[0].last_run_at.is_none());

    let first = store
        .record_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id.clone(),
            status: "success".into(),
            detail: Some("ok".into()),
        })
        .unwrap();
    let second = store
        .record_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id.clone(),
            status: "failed".into(),
            detail: Some("timeout".into()),
        })
        .unwrap();
    let third = store
        .record_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id.clone(),
            status: "success".into(),
            detail: None,
        })
        .unwrap();
    assert!(first.id < second.id && second.id < third.id);
    assert_eq!(third.workflow_id, workflow.id);
    assert!(third.finished_at.is_some());

    let latest = store.list_workflow_runs(&workflow.id, 2).unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(latest[0].id, third.id);
    assert_eq!(latest[1].id, second.id);
    assert_eq!(store.list_workflow_runs(&workflow.id, 10).unwrap().len(), 3);

    let refreshed = store.list_workflows(&project.id).unwrap();
    assert!(
        refreshed[0].last_run_at.is_some(),
        "record_workflow_run doit avancer last_run_at"
    );

    let mut renamed = workflow.clone();
    renamed.name = "veille-renommee".into();
    renamed.steps = json!([{ "kind": "summarize" }]);
    renamed.last_run_at = None;
    store.upsert_workflow(&renamed).unwrap();
    let refreshed = store.list_workflows(&project.id).unwrap();
    assert_eq!(refreshed[0].name, "veille-renommee");
    assert_eq!(refreshed[0].steps, json!([{ "kind": "summarize" }]));
    assert_eq!(refreshed[0].created_at, workflow.created_at);
    assert!(
        refreshed[0].last_run_at.is_some(),
        "l'upsert ne doit pas écraser last_run_at"
    );

    let error = store
        .record_workflow_run(&NewWorkflowRun {
            workflow_id: "wf-inconnu".into(),
            status: "success".into(),
            detail: None,
        })
        .unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 8. Audit
// -------------------------------------------------------------------------

#[test]
fn audit_ordre_decroissant_et_limite() {
    let store = Store::open_in_memory().unwrap();
    for (index, decision) in ["approved", "denied", "auto"].iter().enumerate() {
        store
            .append_audit(&NewAuditEntry {
                category: "shell".into(),
                summary: format!("audit-{index}"),
                detail: Some(format!("commande {index}")),
                decision: (*decision).into(),
            })
            .unwrap();
    }

    let recent = store.list_audit(2).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].summary, "audit-2");
    assert_eq!(recent[1].summary, "audit-1");
    assert!(recent[0].id > recent[1].id);

    let all = store.list_audit(100).unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.windows(2).all(|pair| pair[0].id > pair[1].id));
    assert_eq!(all[0].decision, "auto");

    let error = store
        .append_audit(&NewAuditEntry {
            category: "shell".into(),
            summary: "invalide".into(),
            detail: None,
            decision: "peut-etre".into(),
        })
        .unwrap_err();
    assert!(matches!(error, IhError::Invalid(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 9. Réglages clé/valeur
// -------------------------------------------------------------------------

#[test]
fn reglages_lecture_ecriture_ecrasement() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.get_setting("theme").unwrap(), None);

    store.set_setting("theme", "dark").unwrap();
    assert_eq!(store.get_setting("theme").unwrap().as_deref(), Some("dark"));

    store.set_setting("theme", "light").unwrap();
    assert_eq!(
        store.get_setting("theme").unwrap().as_deref(),
        Some("light"),
        "set_setting doit écraser la valeur existante"
    );
}

// -------------------------------------------------------------------------
// 10. Concurrence
// -------------------------------------------------------------------------

#[test]
fn ecritures_concurrentes_sans_database_locked() {
    const WORKERS: usize = 4;
    const PER_WORKER: usize = 25;

    let db = TempDb::new();
    let store = Arc::new(Store::open(&db.path()).unwrap());
    let project = seed_project(&store, "concurrence");
    let thread = store.create_thread(&project.id, "Parallèle", None).unwrap();
    let barrier = Arc::new(Barrier::new(WORKERS));

    let mut handles = Vec::with_capacity(WORKERS);
    for worker in 0..WORKERS {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let thread_id = thread.id.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for index in 0..PER_WORKER {
                store
                    .append_message(&new_message(
                        &thread_id,
                        "user",
                        format!("worker-{worker}-message-{index}"),
                    ))
                    .expect("écriture concurrente sans database is locked");
            }
        }));
    }
    for handle in handles {
        handle.join().expect("thread d'écriture");
    }

    let messages = store.list_messages(&thread.id, u32::MAX, 0).unwrap();
    assert_eq!(messages.len(), WORKERS * PER_WORKER);
    let unique = messages
        .iter()
        .map(|message| message.id.as_str().to_owned())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), messages.len());
}

// -------------------------------------------------------------------------
// 11. Sessions d'agent
// -------------------------------------------------------------------------

#[test]
fn sessions_cycle_de_vie_et_suppression_du_thread() {
    let store = Store::open_in_memory().unwrap();
    let project = seed_project(&store, "sessions");
    let thread = store
        .create_thread(&project.id, "Session liée", None)
        .unwrap();

    let session = store
        .create_session(&NewSession {
            project_id: project.id.clone(),
            agent_id: "claude-code".into(),
            thread_id: Some(thread.id.clone()),
            prompt: "Analyse le dépôt".into(),
        })
        .unwrap();
    assert_eq!(session.state, AgentState::Starting);
    assert!(session.ended_at.is_none());
    assert_eq!(session.thread_id.as_ref(), Some(&thread.id));

    store
        .update_session_state(&session.id, AgentState::Running, None)
        .unwrap();
    let running = store.list_sessions(&project.id, 10).unwrap().remove(0);
    assert_eq!(running.state, AgentState::Running);
    assert!(running.ended_at.is_none());

    store
        .update_session_state(&session.id, AgentState::Completed, Some(0))
        .unwrap();
    let done = store.list_sessions(&project.id, 10).unwrap().remove(0);
    assert_eq!(done.state, AgentState::Completed);
    assert_eq!(done.exit_code, Some(0));
    assert!(done.ended_at.is_some());

    store.delete_thread(&thread.id).unwrap();
    let after_thread_deletion = store.list_sessions(&project.id, 10).unwrap();
    assert_eq!(
        after_thread_deletion.len(),
        1,
        "ON DELETE SET NULL doit conserver la session"
    );
    assert!(after_thread_deletion[0].thread_id.is_none());

    let error = store
        .update_session_state(&SessionId::new(), AgentState::Failed, None)
        .unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");
}

// -------------------------------------------------------------------------
// 11b. MCP : serveurs, catalogue d'outils et appels
// -------------------------------------------------------------------------

fn new_mcp_server(id: &str) -> McpServerRecord {
    McpServerRecord {
        id: id.into(),
        name: "Serveur test".into(),
        url: "https://mcp.exemple.com/mcp".into(),
        transport: "streamable_http".into(),
        headers: std::collections::BTreeMap::new(),
        timeout_secs: 30,
        enabled: true,
        allow_auto_approve: false,
        has_secret: false,
        last_status: "disconnected".into(),
        last_error: None,
        last_connected_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn new_mcp_tool(server_id: &str, name: &str, risk: &str) -> McpToolRecord {
    McpToolRecord {
        server_id: server_id.into(),
        name: name.into(),
        description: format!("outil {name}"),
        input_schema: json!({"type": "object"}),
        output_schema: None,
        category: "read".into(),
        risk: risk.into(),
        enabled: false,
        discovered_at: Utc::now(),
    }
}

#[test]
fn mcp_serveurs_crud_et_etat() {
    let db = TempDb::new();
    let store = Store::open(&db.path()).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);

    let mut server = new_mcp_server("srv-1");
    store.upsert_mcp_server(&server).unwrap();
    let listed = store.list_mcp_servers().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "Serveur test");
    assert_eq!(listed[0].transport, "streamable_http");

    // Mise à jour : created_at préservé, champs remplacés.
    server.name = "Renommé".into();
    server.timeout_secs = 60;
    server.allow_auto_approve = true;
    store.upsert_mcp_server(&server).unwrap();
    let refreshed = store.get_mcp_server("srv-1").unwrap().unwrap();
    assert_eq!(refreshed.name, "Renommé");
    assert_eq!(refreshed.timeout_secs, 60);
    assert!(refreshed.allow_auto_approve);

    store
        .set_mcp_server_status("srv-1", "connected", None, Some(Utc::now()))
        .unwrap();
    let refreshed = store.get_mcp_server("srv-1").unwrap().unwrap();
    assert_eq!(refreshed.last_status, "connected");
    assert!(refreshed.last_connected_at.is_some());

    let error = store
        .set_mcp_server_status("srv-1", "explosion", None, None)
        .unwrap_err();
    assert!(matches!(error, IhError::Invalid(_)), "{error:?}");

    let mut bad = new_mcp_server("srv-bad");
    bad.transport = "websocket".into();
    assert!(store.upsert_mcp_server(&bad).is_err());
    bad.transport = "sse".into();
    bad.timeout_secs = 0;
    assert!(store.upsert_mcp_server(&bad).is_err());

    store.delete_mcp_server("srv-1").unwrap();
    assert!(store.get_mcp_server("srv-1").unwrap().is_none());
    assert!(store.delete_mcp_server("srv-1").is_err());
}

#[test]
fn mcp_catalogue_preserve_activation_et_purge_les_outils_retires() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_mcp_server(&new_mcp_server("srv")).unwrap();

    store
        .refresh_mcp_tools(
            "srv",
            &[
                new_mcp_tool("srv", "read_file", "low"),
                new_mcp_tool("srv", "delete_all", "critical"),
            ],
        )
        .unwrap();
    let tools = store.list_mcp_tools(Some("srv")).unwrap();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| !tool.enabled));

    store
        .set_mcp_tool_enabled("srv", "read_file", true)
        .unwrap();
    let error = store
        .set_mcp_tool_enabled("srv", "inconnu", true)
        .unwrap_err();
    assert!(matches!(error, IhError::NotFound(_)), "{error:?}");

    // Rafraîchissement : read_file garde son activation, delete_all disparaît.
    store
        .refresh_mcp_tools("srv", &[new_mcp_tool("srv", "read_file", "low")])
        .unwrap();
    let tools = store.list_mcp_tools(Some("srv")).unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "read_file");
    assert!(tools[0].enabled, "l'activation doit être préservée");
    let gone = store.get_mcp_tool("srv", "delete_all").unwrap();
    assert!(gone.is_none());

    // Un outil nouvellement apparu est désactivé par défaut.
    store
        .refresh_mcp_tools(
            "srv",
            &[
                new_mcp_tool("srv", "read_file", "low"),
                new_mcp_tool("srv", "send_mail", "medium"),
            ],
        )
        .unwrap();
    let tools = store.list_mcp_tools(Some("srv")).unwrap();
    let mail = tools.iter().find(|tool| tool.name == "send_mail").unwrap();
    assert!(!mail.enabled);
}

#[test]
fn mcp_appels_historises_et_cascade() {
    let store = Store::open_in_memory().unwrap();
    store.upsert_mcp_server(&new_mcp_server("srv")).unwrap();

    let first = store
        .record_mcp_call(&NewMcpCall {
            server_id: "srv".into(),
            tool_name: "read_file".into(),
            caller: "ui".into(),
            caller_id: None,
            arguments: json!({"path": "a.txt"}),
            status: "completed".into(),
            error: None,
            duration_ms: Some(12),
        })
        .unwrap();
    let second = store
        .record_mcp_call(&NewMcpCall {
            server_id: "srv".into(),
            tool_name: "delete_all".into(),
            caller: "automation".into(),
            caller_id: Some("wf-1".into()),
            arguments: json!({}),
            status: "denied".into(),
            error: Some("approbation absente".into()),
            duration_ms: None,
        })
        .unwrap();
    assert!(first.id < second.id);
    assert_eq!(second.caller, "automation");
    assert_eq!(second.caller_id.as_deref(), Some("wf-1"));
    assert_eq!(second.error.as_deref(), Some("approbation absente"));

    let all = store.list_mcp_calls(10, None).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, second.id);
    let scoped = store.list_mcp_calls(10, Some("srv")).unwrap();
    assert_eq!(scoped.len(), 2);

    let error = store
        .record_mcp_call(&NewMcpCall {
            server_id: "srv".into(),
            tool_name: "x".into(),
            caller: "robot".into(),
            caller_id: None,
            arguments: json!({}),
            status: "completed".into(),
            error: None,
            duration_ms: None,
        })
        .unwrap_err();
    assert!(matches!(error, IhError::Invalid(_)), "{error:?}");

    store.delete_mcp_server("srv").unwrap();
    assert!(store.list_mcp_calls(10, None).unwrap().is_empty());
    assert!(store.list_mcp_tools(Some("srv")).unwrap().is_empty());
}

// -------------------------------------------------------------------------
// 12. Send + Sync pour l'état partagé Tauri
// -------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn store_est_send_et_sync() {
    assert_send_sync::<Store>();
}

// -------------------------------------------------------------------------
// Appels d'outils proposés par un modèle (chat natif, migration v6)
// -------------------------------------------------------------------------

fn new_chat_tool_call(
    thread: &Thread,
    project: &Project,
    request_id: &str,
    status: &str,
) -> NewChatToolCall {
    NewChatToolCall {
        thread_id: thread.id.clone(),
        project_id: project.id.clone(),
        request_id: request_id.to_owned(),
        round: 0,
        provider_call_id: "call_1".to_owned(),
        exposed_name: "demo__echo".to_owned(),
        server_id: "demo".to_owned(),
        tool: "echo".to_owned(),
        risk: "low".to_owned(),
        category: "read".to_owned(),
        fingerprint: "f".repeat(64),
        arguments: json!({ "text": "[masqué]" }),
        raw_arguments: json!({ "text": "valeur réelle" }),
        status: status.to_owned(),
        error: None,
    }
}

#[test]
fn chat_tool_call_proposition_et_lectures() {
    let store = Store::open_in_memory().expect("base mémoire");
    let project = seed_project(&store, "chat-tools");
    let thread = store
        .create_thread(&project.id, "Essai", None)
        .expect("conversation");
    let pending = store
        .insert_chat_tool_call(&new_chat_tool_call(&thread, &project, "req-1", "pending"))
        .expect("proposition pending");
    assert_eq!(pending.status, "pending");
    assert_eq!(pending.exposed_name, "demo__echo");
    assert!(pending.decided_at.is_none());

    let by_thread = store.list_chat_tool_calls(&thread.id).expect("liste");
    assert_eq!(by_thread.len(), 1);
    assert_eq!(by_thread[0].id, pending.id);
    let by_request = store
        .list_chat_tool_calls_for_request("req-1")
        .expect("liste par tour");
    assert_eq!(by_request.len(), 1);
    assert_eq!(by_request[0].server_id, "demo");

    let invalid = store
        .insert_chat_tool_call(&new_chat_tool_call(&thread, &project, "req-1", "invalid"))
        .expect("proposition invalid");
    assert_eq!(invalid.status, "invalid");
    assert!(store
        .insert_chat_tool_call(&new_chat_tool_call(&thread, &project, "req-1", "completed"))
        .is_err());
    assert!(store
        .list_chat_tool_calls(&ThreadId::new())
        .expect("liste vide")
        .is_empty());
}

#[test]
fn chat_tool_call_decisions_et_transitions_verrouillees() {
    let store = Store::open_in_memory().expect("base mémoire");
    let project = seed_project(&store, "chat-tools");
    let thread = store
        .create_thread(&project.id, "Essai", None)
        .expect("conversation");
    let call = store
        .insert_chat_tool_call(&new_chat_tool_call(&thread, &project, "req-1", "pending"))
        .expect("proposition");

    let denied = store
        .decide_chat_tool_call(&call.id, "denied", None)
        .expect("refus");
    assert_eq!(denied.status, "denied");
    assert!(denied.decided_at.is_some());
    // Une proposition déjà décidée ne change plus.
    assert!(matches!(
        store.decide_chat_tool_call(&call.id, "invalid", None),
        Err(IhError::Conflict(_))
    ));

    // Une proposition refusée ne peut plus être exécutée : la fin exige une
    // proposition encore ouverte (réclamée par une approbation).
    assert!(matches!(
        store.finish_chat_tool_call(&call.id, "completed", None, Some("résultat"), None),
        Err(IhError::Conflict(_))
    ));
}

#[test]
fn chat_tool_call_reclamation_atomique_une_seule_execution() {
    let store = Store::open_in_memory().expect("base mémoire");
    let project = seed_project(&store, "chat-tools");
    let thread = store
        .create_thread(&project.id, "Essai", None)
        .expect("conversation");
    let call = store
        .insert_chat_tool_call(&new_chat_tool_call(&thread, &project, "req-1", "pending"))
        .expect("proposition");

    assert!(store
        .claim_chat_tool_call(&call.id, "appro-1")
        .expect("première réclamation"));
    // Une seconde réclamation échoue : une seule exécution possible.
    assert!(!store
        .claim_chat_tool_call(&call.id, "appro-2")
        .expect("seconde réclamation"));
    // Un refus après réclamation est impossible.
    assert!(matches!(
        store.decide_chat_tool_call(&call.id, "denied", None),
        Err(IhError::Conflict(_))
    ));
    let finished = store
        .finish_chat_tool_call(
            &call.id,
            "completed",
            None,
            Some("résultat"),
            Some("appro-1"),
        )
        .expect("fin");
    assert_eq!(finished.status, "completed");
    assert_eq!(finished.result_text.as_deref(), Some("résultat"));
    assert_eq!(finished.approval_id.as_deref(), Some("appro-1"));
    assert!(finished.finished_at.is_some());
    // Une seconde fin échoue.
    assert!(matches!(
        store.finish_chat_tool_call(&call.id, "failed", None, None, None),
        Err(IhError::Conflict(_))
    ));
}

#[test]
fn chat_tool_call_pending_n_est_pas_rejoue_apres_reouverture() {
    let db = TempDb::new();
    let project = {
        let store = Store::open(&db.path()).expect("ouverture");
        let project = seed_project(&store, "chat-tools");
        let thread = store
            .create_thread(&project.id, "Essai", None)
            .expect("conversation");
        store
            .insert_chat_tool_call(&new_chat_tool_call(&thread, &project, "req-1", "pending"))
            .expect("proposition");
        store
    };
    drop(project);

    // Nouvelle ouverture : la proposition reste en attente, jamais exécutée.
    let store = Store::open(&db.path()).expect("réouverture");
    let thread_id = store
        .list_threads(&store.list_projects().unwrap()[0].id)
        .unwrap()[0]
        .id
        .clone();
    let calls = store.list_chat_tool_calls(&thread_id).expect("liste");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].status, "pending");
    assert!(calls[0].finished_at.is_none());
}

// -------------------------------------------------------------------------
// Protection des données : sauvegarde cohérente (WAL compris) avant migration
// -------------------------------------------------------------------------

#[test]
fn sauvegarde_avant_migration_preserve_les_donnees() {
    let db = TempDb::new();
    let path = db.path();
    let project;
    {
        let store = Store::open(&path).expect("création");
        project = seed_project(&store, "sauvegarde");
        // État d'une base 0.6.1 : version 5, sans la table v6.
        let conn = rusqlite::Connection::open(&path).expect("connexion directe");
        conn.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE);
             DROP TABLE chat_tool_calls;
             PRAGMA user_version = 5;",
        )
        .expect("simulation du schéma v5");
    }

    let store = Store::open(&path).expect("réouverture");
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    let preserved = store
        .get_project(&project.id)
        .expect("lecture")
        .expect("projet");
    assert_eq!(preserved.name, "sauvegarde");

    let backups: Vec<std::path::PathBuf> = std::fs::read_dir(path.parent().unwrap())
        .expect("lecture du dossier")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| {
            entry
                .file_name()
                .map(|name| {
                    name.to_string_lossy().contains(".pre-v5-")
                        && name.to_string_lossy().ends_with(".bak")
                })
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(backups.len(), 1, "une sauvegarde avant migration attendue");
    // La copie contient réellement les données d'avant migration.
    let copy = rusqlite::Connection::open_with_flags(
        &backups[0],
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("ouverture de la sauvegarde");
    let name: String = copy
        .query_row(
            "SELECT name FROM projects WHERE id = ?1",
            [project.id.as_str()],
            |row| row.get(0),
        )
        .expect("projet dans la sauvegarde");
    assert_eq!(name, "sauvegarde");
    let version: i64 = copy
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version de la sauvegarde");
    assert_eq!(version, 5, "la sauvegarde garde le schéma d'origine");
}

#[test]
fn aucune_sauvegarde_quand_le_schema_est_a_jour() {
    let db = TempDb::new();
    let path = db.path();
    drop(Store::open(&path).expect("création"));
    drop(Store::open(&path).expect("réouverture"));
    let backups = std::fs::read_dir(path.parent().unwrap())
        .expect("lecture du dossier")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| entry.to_string_lossy().ends_with(".bak"))
        .count();
    assert_eq!(backups, 0, "aucune sauvegarde sans migration");
}
