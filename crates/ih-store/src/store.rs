//! Connexion SQLite et opérations de persistance d'IH.
//!
//! # Concurrence
//!
//! Une seule connexion est partagée derrière un [`Mutex`] (état Tauri
//! `Arc<Store>`). Les écritures d'un même processus sont donc sérialisées par
//! le mutex, et `busy_timeout` couvre le cas d'un autre processus (instance
//! secondaire, outil d'inspection) qui tiendrait brièvement le verrou WAL.
//! Aucune opération ne conserve le verrou au-delà de la requête.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use ih_core::events::{AgentState, TaskState};
use ih_core::{AgentId, IhError, IhResult, MessageId, ProjectId, SessionId, TaskId, ThreadId};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::migrate;
use crate::model::{
    AuditEntry, InstalledExtension, MemoryEntry, Message, NewAuditEntry, NewMemoryEntry,
    NewMessage, NewSession, NewTask, NewWorkflowRun, Project, SessionRow, Task, Thread, Workflow,
    WorkflowRun,
};
use crate::util::{
    agent_state_str, db_err, decode_json, decode_opt_json, encode_json, get_agent_state, get_id,
    get_opt_id, get_task_state, get_u32, like_pattern, require_non_empty, task_state_str,
    validate_choice,
};

const MESSAGE_ROLES: [&str; 4] = ["user", "assistant", "system", "tool"];
const TASK_ROLES: [&str; 3] = ["lead", "worker", "reviewer"];
const MEMORY_SCOPES: [&str; 2] = ["project", "global"];
const EXTENSION_KINDS: [&str; 2] = ["skill", "plugin"];
const AUDIT_DECISIONS: [&str; 3] = ["approved", "denied", "auto"];

const LIST_MEMORY_ALL: &str =
    "SELECT id, project_id, scope, key, content, tags, source, created_at, updated_at
     FROM memory
     ORDER BY updated_at DESC, rowid DESC";
const LIST_MEMORY_PROJECT: &str =
    "SELECT id, project_id, scope, key, content, tags, source, created_at, updated_at
     FROM memory
     WHERE project_id = ?1
     ORDER BY updated_at DESC, rowid DESC";
const SEARCH_MEMORY_ALL: &str =
    "SELECT id, project_id, scope, key, content, tags, source, created_at, updated_at
     FROM memory
     WHERE (content LIKE ?1 ESCAPE '\\'
            OR IFNULL(key, '') LIKE ?1 ESCAPE '\\'
            OR tags LIKE ?1 ESCAPE '\\')
     ORDER BY updated_at DESC, rowid DESC
     LIMIT ?2";
const SEARCH_MEMORY_PROJECT: &str =
    "SELECT id, project_id, scope, key, content, tags, source, created_at, updated_at
     FROM memory
     WHERE project_id = ?1
       AND (content LIKE ?2 ESCAPE '\\'
            OR IFNULL(key, '') LIKE ?2 ESCAPE '\\'
            OR tags LIKE ?2 ESCAPE '\\')
     ORDER BY updated_at DESC, rowid DESC
     LIMIT ?3";

/// Délai d'attente du verrou SQLite avant de renvoyer `database is locked`.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Persistance SQLite d'IH.
///
/// `Store` est `Send + Sync` : il peut vivre dans un état Tauri partagé et être
/// utilisé depuis plusieurs tâches asynchrones ou threads.
pub struct Store {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl Store {
    /// Ouvre (ou crée) la base au chemin donné.
    ///
    /// Crée les dossiers manquants, active WAL, les clés étrangères et un
    /// `busy_timeout`, puis applique les migrations manquantes.
    pub fn open(path: &Path) -> IhResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)
            .map_err(|err| db_err(format!("ouverture de {}", path.display()), err))?;
        Self::from_connection(conn, Some(path.to_path_buf()))
    }

    /// Ouvre une base éphémère en mémoire, utilisée par les tests et les
    /// aperçus. Aucun fichier n'est créé.
    pub fn open_in_memory() -> IhResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|err| db_err("ouverture d'une base SQLite en mémoire", err))?;
        Self::from_connection(conn, None)
    }

    fn from_connection(conn: Connection, path: Option<PathBuf>) -> IhResult<Self> {
        configure(&conn)?;
        let store = Self {
            conn: Mutex::new(conn),
            path,
        };
        {
            let mut conn = store.lock()?;
            migrate::migrate(&mut conn)?;
        }
        Ok(store)
    }

    /// Version courante du schéma (valeur de `PRAGMA user_version`).
    pub fn schema_version(&self) -> IhResult<u32> {
        let conn = self.lock()?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|err| db_err("lecture de PRAGMA user_version", err))?;
        u32::try_from(version).map_err(|_| {
            IhError::Db(format!(
                "PRAGMA user_version invalide (négatif) : {version}"
            ))
        })
    }

    /// Chemin du fichier de base, ou `None` pour une base en mémoire.
    pub fn db_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn lock(&self) -> IhResult<MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| {
            IhError::Internal("verrou de connexion SQLite empoisonné par un panic".into())
        })
    }

    // ---------------------------------------------------------------------
    // Réglages clé/valeur
    // ---------------------------------------------------------------------

    /// Lit un réglage, ou `None` s'il n'a jamais été écrit.
    pub fn get_setting(&self, key: &str) -> IhResult<Option<String>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| db_err(format!("lecture du réglage « {key} »"), err))
    }

    /// Écrit ou remplace un réglage.
    pub fn set_setting(&self, key: &str, value: &str) -> IhResult<()> {
        require_non_empty("clé de réglage", key)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value,
                                            updated_at = excluded.updated_at",
            params![key, value, Utc::now()],
        )
        .map_err(|err| db_err(format!("écriture du réglage « {key} »"), err))?;
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Projets
    // ---------------------------------------------------------------------

    /// Insère un projet ou met à jour ses métadonnées sans doublon.
    pub fn upsert_project(&self, project: &Project) -> IhResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO projects (id, path, name, created_at, last_opened_at, trusted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (id) DO UPDATE SET path = excluded.path,
                                            name = excluded.name,
                                            last_opened_at = excluded.last_opened_at,
                                            trusted = excluded.trusted",
            params![
                project.id.as_str(),
                project.path,
                project.name,
                project.created_at,
                project.last_opened_at,
                project.trusted,
            ],
        )
        .map_err(|err| db_err(format!("enregistrement du projet {}", project.id), err))?;
        Ok(())
    }

    /// Liste les projets connus, du plus récemment ouvert au plus ancien.
    pub fn list_projects(&self) -> IhResult<Vec<Project>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, path, name, created_at, last_opened_at, trusted
                 FROM projects
                 ORDER BY last_opened_at DESC, rowid DESC",
            )
            .map_err(|err| db_err("préparation de la liste des projets", err))?;
        let rows = stmt
            .query_map([], project_from_row)
            .map_err(|err| db_err("lecture des projets", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture des projets", err))
    }

    /// Lit un projet par identifiant ; `None` s'il n'existe pas.
    pub fn get_project(&self, id: &ProjectId) -> IhResult<Option<Project>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, path, name, created_at, last_opened_at, trusted
             FROM projects WHERE id = ?1",
            params![id.as_str()],
            project_from_row,
        )
        .optional()
        .map_err(|err| db_err(format!("lecture du projet {id}"), err))
    }

    /// Supprime les métadonnées IH d'un projet.
    ///
    /// Les conversations, messages, sessions, tâches, mémoires et workflows
    /// rattachés sont supprimés en cascade par SQLite. Aucun fichier du projet
    /// sur le disque n'est touché.
    pub fn remove_project(&self, id: &ProjectId) -> IhResult<()> {
        let conn = self.lock()?;
        let removed = conn
            .execute("DELETE FROM projects WHERE id = ?1", params![id.as_str()])
            .map_err(|err| db_err(format!("suppression du projet {id}"), err))?;
        if removed == 0 {
            return Err(IhError::NotFound(format!("projet {id}")));
        }
        Ok(())
    }

    /// Met à jour la date de dernière ouverture d'un projet.
    pub fn touch_project(&self, id: &ProjectId, at: DateTime<Utc>) -> IhResult<()> {
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE projects SET last_opened_at = ?2 WHERE id = ?1",
                params![id.as_str(), at],
            )
            .map_err(|err| db_err(format!("mise à jour du projet {id}"), err))?;
        if updated == 0 {
            return Err(IhError::NotFound(format!("projet {id}")));
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Conversations et messages
    // ---------------------------------------------------------------------

    /// Crée une conversation rattachée à un projet.
    pub fn create_thread(
        &self,
        project_id: &ProjectId,
        title: &str,
        agent_id: Option<&str>,
    ) -> IhResult<Thread> {
        if let Some(agent_id) = agent_id {
            AgentId::parse(agent_id)?;
        }
        let id = ThreadId::new();
        let now = Utc::now();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO threads (id, project_id, title, agent_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id.as_str(), project_id.as_str(), title, agent_id, now],
        )
        .map_err(|err| {
            db_err(
                format!("création de la conversation pour {project_id}"),
                err,
            )
        })?;
        Ok(Thread {
            id,
            project_id: project_id.clone(),
            title: title.to_owned(),
            agent_id: agent_id.map(str::to_owned),
            created_at: now,
            updated_at: now,
        })
    }

    /// Liste les conversations d'un projet, de la plus récemment active à la
    /// plus ancienne.
    pub fn list_threads(&self, project_id: &ProjectId) -> IhResult<Vec<Thread>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, title, agent_id, created_at, updated_at
                 FROM threads
                 WHERE project_id = ?1
                 ORDER BY updated_at DESC, rowid DESC",
            )
            .map_err(|err| db_err("préparation de la liste des conversations", err))?;
        let rows = stmt
            .query_map(params![project_id.as_str()], thread_from_row)
            .map_err(|err| db_err("lecture des conversations", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture des conversations", err))
    }

    /// Lit une conversation par identifiant.
    pub fn get_thread(&self, id: &ThreadId) -> IhResult<Option<Thread>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, project_id, title, agent_id, created_at, updated_at
             FROM threads WHERE id = ?1",
            params![id.as_str()],
            thread_from_row,
        )
        .optional()
        .map_err(|err| db_err(format!("lecture de la conversation {id}"), err))
    }

    /// Renomme une conversation et met à jour son horodatage.
    pub fn rename_thread(&self, id: &ThreadId, title: &str) -> IhResult<()> {
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE threads SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.as_str(), title, Utc::now()],
            )
            .map_err(|err| db_err(format!("renommage de la conversation {id}"), err))?;
        if updated == 0 {
            return Err(IhError::NotFound(format!("conversation {id}")));
        }
        Ok(())
    }

    /// Supprime une conversation ; ses messages sont supprimés en cascade.
    pub fn delete_thread(&self, id: &ThreadId) -> IhResult<()> {
        let conn = self.lock()?;
        let deleted = conn
            .execute("DELETE FROM threads WHERE id = ?1", params![id.as_str()])
            .map_err(|err| db_err(format!("suppression de la conversation {id}"), err))?;
        if deleted == 0 {
            return Err(IhError::NotFound(format!("conversation {id}")));
        }
        Ok(())
    }

    /// Ajoute un message et rafraîchit `threads.updated_at` dans la même
    /// transaction.
    pub fn append_message(&self, msg: &NewMessage) -> IhResult<Message> {
        validate_choice("rôle de message", &msg.role, &MESSAGE_ROLES)?;
        let metadata = match &msg.metadata {
            Some(value) => Some(encode_json("métadonnées du message", value)?),
            None => None,
        };
        let id = MessageId::new();
        let now = Utc::now();
        let mut conn = self.lock()?;
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de l'insertion du message", err))?;
        tx.execute(
            "INSERT INTO messages (id, thread_id, role, content, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.as_str(),
                msg.thread_id.as_str(),
                msg.role,
                msg.content,
                metadata,
                now,
            ],
        )
        .map_err(|err| db_err(format!("insertion du message dans {}", msg.thread_id), err))?;
        let touched = tx
            .execute(
                "UPDATE threads SET updated_at = ?2 WHERE id = ?1",
                params![msg.thread_id.as_str(), now],
            )
            .map_err(|err| {
                db_err(
                    format!("mise à jour de la conversation {}", msg.thread_id),
                    err,
                )
            })?;
        if touched == 0 {
            return Err(IhError::NotFound(format!("conversation {}", msg.thread_id)));
        }
        tx.commit()
            .map_err(|err| db_err("validation de l'insertion du message", err))?;
        Ok(Message {
            id,
            thread_id: msg.thread_id.clone(),
            role: msg.role.clone(),
            content: msg.content.clone(),
            metadata: msg.metadata.clone(),
            created_at: now,
        })
    }

    /// Liste les messages d'une conversation dans l'ordre chronologique
    /// (insertion en cas d'horodatages identiques).
    pub fn list_messages(
        &self,
        thread_id: &ThreadId,
        limit: u32,
        offset: u32,
    ) -> IhResult<Vec<Message>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, thread_id, role, content, metadata, created_at
                 FROM messages
                 WHERE thread_id = ?1
                 ORDER BY created_at ASC, rowid ASC
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|err| db_err("préparation de la lecture des messages", err))?;
        let rows = stmt
            .query_map(
                params![thread_id.as_str(), i64::from(limit), i64::from(offset)],
                message_from_row,
            )
            .map_err(|err| db_err(format!("lecture des messages de {thread_id}"), err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err(format!("lecture des messages de {thread_id}"), err))
    }

    // ---------------------------------------------------------------------
    // Sessions d'agent
    // ---------------------------------------------------------------------

    /// Crée une session d'agent à l'état `starting`.
    pub fn create_session(&self, new: &NewSession) -> IhResult<SessionRow> {
        AgentId::parse(new.agent_id.as_str())?;
        let id = SessionId::new();
        let now = Utc::now();
        let state = AgentState::Starting;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO sessions
                (id, project_id, agent_id, thread_id, prompt, state, exit_code, created_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL)",
            params![
                id.as_str(),
                new.project_id.as_str(),
                new.agent_id,
                new.thread_id.as_ref().map(ThreadId::as_str),
                new.prompt,
                agent_state_str(state),
                now,
            ],
        )
        .map_err(|err| db_err(format!("création de la session pour {}", new.project_id), err))?;
        Ok(SessionRow {
            id,
            project_id: new.project_id.clone(),
            agent_id: new.agent_id.clone(),
            thread_id: new.thread_id.clone(),
            state,
            exit_code: None,
            created_at: now,
            ended_at: None,
        })
    }

    /// Met à jour l'état d'une session ; un état terminal renseigne `ended_at`.
    ///
    /// La politesse des transitions (`AgentState::can_transition_to`) reste de
    /// la responsabilité de l'orchestrateur : le store enregistre l'état reçu.
    pub fn update_session_state(
        &self,
        id: &SessionId,
        state: AgentState,
        exit_code: Option<i32>,
    ) -> IhResult<()> {
        let ended_at = state.is_terminal().then(Utc::now);
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE sessions
                 SET state = ?2, exit_code = ?3, ended_at = COALESCE(?4, ended_at)
                 WHERE id = ?1",
                params![id.as_str(), agent_state_str(state), exit_code, ended_at],
            )
            .map_err(|err| db_err(format!("mise à jour de la session {id}"), err))?;
        if updated == 0 {
            return Err(IhError::NotFound(format!("session {id}")));
        }
        Ok(())
    }

    /// Liste les sessions d'un projet, de la plus récente à la plus ancienne.
    pub fn list_sessions(&self, project_id: &ProjectId, limit: u32) -> IhResult<Vec<SessionRow>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, agent_id, thread_id, state, exit_code, created_at, ended_at
                 FROM sessions
                 WHERE project_id = ?1
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?2",
            )
            .map_err(|err| db_err("préparation de la liste des sessions", err))?;
        let rows = stmt
            .query_map(
                params![project_id.as_str(), i64::from(limit)],
                session_from_row,
            )
            .map_err(|err| db_err(format!("lecture des sessions de {project_id}"), err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err(format!("lecture des sessions de {project_id}"), err))
    }

    // ---------------------------------------------------------------------
    // Tâches
    // ---------------------------------------------------------------------

    /// Crée une tâche à l'état `pending`.
    ///
    /// `depends_on` est sérialisé en JSON dans `tasks.depends_on` : l'ensemble
    /// est relu d'un bloc et n'est jamais joint par SQL.
    pub fn create_task(&self, new: &NewTask) -> IhResult<Task> {
        validate_choice("rôle de tâche", &new.role, &TASK_ROLES)?;
        let depends_on = encode_json("dépendances de la tâche", &new.depends_on)?;
        let id = TaskId::new();
        let now = Utc::now();
        let state = TaskState::Pending;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO tasks
                (id, project_id, title, description, state, assignee, depends_on, role,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                id.as_str(),
                new.project_id.as_str(),
                new.title,
                new.description,
                task_state_str(state),
                new.assignee,
                depends_on,
                new.role,
                now,
            ],
        )
        .map_err(|err| db_err(format!("création de la tâche pour {}", new.project_id), err))?;
        Ok(Task {
            id,
            project_id: new.project_id.clone(),
            title: new.title.clone(),
            description: new.description.clone(),
            state,
            assignee: new.assignee.clone(),
            depends_on: new.depends_on.clone(),
            role: new.role.clone(),
            created_at: now,
            updated_at: now,
        })
    }

    /// Change l'état d'une tâche et met à jour son horodatage.
    pub fn update_task_state(&self, id: &TaskId, state: TaskState) -> IhResult<()> {
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE tasks SET state = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.as_str(), task_state_str(state), Utc::now()],
            )
            .map_err(|err| db_err(format!("mise à jour de la tâche {id}"), err))?;
        if updated == 0 {
            return Err(IhError::NotFound(format!("tâche {id}")));
        }
        Ok(())
    }

    /// Liste les tâches d'un projet, dans l'ordre de création.
    pub fn list_tasks(&self, project_id: &ProjectId) -> IhResult<Vec<Task>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, title, description, state, assignee, depends_on, role,
                        created_at, updated_at
                 FROM tasks
                 WHERE project_id = ?1
                 ORDER BY created_at ASC, rowid ASC",
            )
            .map_err(|err| db_err("préparation de la liste des tâches", err))?;
        let rows = stmt
            .query_map(params![project_id.as_str()], task_from_row)
            .map_err(|err| db_err(format!("lecture des tâches de {project_id}"), err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err(format!("lecture des tâches de {project_id}"), err))
    }

    /// Lit une tâche par identifiant.
    pub fn get_task(&self, id: &TaskId) -> IhResult<Option<Task>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT id, project_id, title, description, state, assignee, depends_on, role,
                    created_at, updated_at
             FROM tasks WHERE id = ?1",
            params![id.as_str()],
            task_from_row,
        )
        .optional()
        .map_err(|err| db_err(format!("lecture de la tâche {id}"), err))
    }

    // ---------------------------------------------------------------------
    // Mémoire
    // ---------------------------------------------------------------------

    /// Ajoute une entrée de mémoire.
    ///
    /// Une entrée `project` exige un `project_id` ; une entrée `global` ne doit
    /// pas en porter. Les tags sont stockés en JSON.
    pub fn add_memory(&self, entry: &NewMemoryEntry) -> IhResult<MemoryEntry> {
        validate_choice("portée de mémoire", &entry.scope, &MEMORY_SCOPES)?;
        match (entry.scope.as_str(), entry.project_id.as_ref()) {
            ("project", None) => {
                return Err(IhError::Invalid(
                    "une mémoire de portée « project » exige project_id".into(),
                ));
            }
            ("global", Some(_)) => {
                return Err(IhError::Invalid(
                    "une mémoire de portée « global » ne doit pas porter project_id".into(),
                ));
            }
            _ => {}
        }
        let tags = encode_json("tags de la mémoire", &entry.tags)?;
        let id = format!("mem-{}", uuid::Uuid::new_v4());
        let now = Utc::now();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO memory
                (id, project_id, scope, key, content, tags, source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                id,
                entry.project_id.as_ref().map(ProjectId::as_str),
                entry.scope,
                entry.key,
                entry.content,
                tags,
                entry.source,
                now,
            ],
        )
        .map_err(|err| db_err("ajout d'une entrée de mémoire", err))?;
        Ok(MemoryEntry {
            id,
            project_id: entry.project_id.clone(),
            scope: entry.scope.clone(),
            key: entry.key.clone(),
            content: entry.content.clone(),
            tags: entry.tags.clone(),
            source: entry.source.clone(),
            created_at: now,
            updated_at: now,
        })
    }

    /// Liste les entrées de mémoire.
    ///
    /// `Some(project)` filtre strictement sur ce projet ; `None` retourne toutes
    /// les entrées (y compris globales), de la plus récemment modifiée à la plus
    /// ancienne.
    pub fn list_memory(&self, project_id: Option<&ProjectId>) -> IhResult<Vec<MemoryEntry>> {
        let conn = self.lock()?;
        let sql = match project_id {
            Some(_) => LIST_MEMORY_PROJECT,
            None => LIST_MEMORY_ALL,
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|err| db_err("préparation de la liste de la mémoire", err))?;
        let rows = match project_id {
            Some(id) => stmt.query_map(params![id.as_str()], memory_from_row),
            None => stmt.query_map([], memory_from_row),
        }
        .map_err(|err| db_err("lecture de la mémoire", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture de la mémoire", err))
    }

    /// Recherche insensible à la casse (ASCII pour `LIKE` SQLite) dans le
    /// contenu, la clé et les tags.
    ///
    /// Le motif est toujours un paramètre lié ; les jokers `%` et `_` saisis
    /// par l'utilisateur sont échappés pour rester littéraux.
    pub fn search_memory(
        &self,
        project_id: Option<&ProjectId>,
        query: &str,
        limit: u32,
    ) -> IhResult<Vec<MemoryEntry>> {
        let pattern = like_pattern(query);
        let limit = i64::from(limit);
        let conn = self.lock()?;
        let sql = match project_id {
            Some(_) => SEARCH_MEMORY_PROJECT,
            None => SEARCH_MEMORY_ALL,
        };
        let mut stmt = conn
            .prepare(sql)
            .map_err(|err| db_err("préparation de la recherche mémoire", err))?;
        let rows = match project_id {
            Some(id) => stmt.query_map(params![id.as_str(), pattern, limit], memory_from_row),
            None => stmt.query_map(params![pattern, limit], memory_from_row),
        }
        .map_err(|err| db_err("recherche dans la mémoire", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("recherche dans la mémoire", err))
    }

    /// Supprime une entrée de mémoire.
    pub fn delete_memory(&self, id: &str) -> IhResult<()> {
        let conn = self.lock()?;
        let deleted = conn
            .execute("DELETE FROM memory WHERE id = ?1", params![id])
            .map_err(|err| db_err(format!("suppression de la mémoire {id}"), err))?;
        if deleted == 0 {
            return Err(IhError::NotFound(format!("mémoire {id}")));
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Extensions installées (skills et plugins MCP)
    // ---------------------------------------------------------------------

    /// Insère ou met à jour une extension.
    ///
    /// La clé est le couple `(id, kind)` : un même identifiant peut exister à
    /// la fois comme skill et comme plugin sans collision. `installed_at` est
    /// conservé lors d'une mise à jour ; `updated_at` vient de l'appelant.
    pub fn upsert_extension(&self, ext: &InstalledExtension) -> IhResult<()> {
        validate_extension_kind(&ext.kind)?;
        require_non_empty("identifiant d'extension", &ext.id)?;
        let manifest = encode_json("manifeste d'extension", &ext.manifest)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO extensions
                (id, kind, name, version, source, manifest, sha256, enabled, installed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (id, kind) DO UPDATE SET name = excluded.name,
                                                  version = excluded.version,
                                                  source = excluded.source,
                                                  manifest = excluded.manifest,
                                                  sha256 = excluded.sha256,
                                                  enabled = excluded.enabled,
                                                  updated_at = excluded.updated_at",
            params![
                ext.id,
                ext.kind,
                ext.name,
                ext.version,
                ext.source,
                manifest,
                ext.sha256,
                ext.enabled,
                ext.installed_at,
                ext.updated_at,
            ],
        )
        .map_err(|err| db_err(format!("enregistrement de l'extension {}", ext.id), err))?;
        Ok(())
    }

    /// Liste les extensions d'un type donné (`skill` ou `plugin`).
    pub fn list_extensions(&self, kind: &str) -> IhResult<Vec<InstalledExtension>> {
        validate_extension_kind(kind)?;
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, name, version, source, manifest, sha256, enabled,
                        installed_at, updated_at
                 FROM extensions
                 WHERE kind = ?1
                 ORDER BY name ASC, id ASC",
            )
            .map_err(|err| db_err("préparation de la liste des extensions", err))?;
        let rows = stmt
            .query_map(params![kind], extension_from_row)
            .map_err(|err| db_err("lecture des extensions", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture des extensions", err))
    }

    /// Supprime une extension identifiée par son couple `(id, kind)`.
    pub fn remove_extension(&self, id: &str, kind: &str) -> IhResult<()> {
        validate_extension_kind(kind)?;
        let conn = self.lock()?;
        let deleted = conn
            .execute(
                "DELETE FROM extensions WHERE id = ?1 AND kind = ?2",
                params![id, kind],
            )
            .map_err(|err| db_err(format!("suppression de l'extension {id}"), err))?;
        if deleted == 0 {
            return Err(IhError::NotFound(format!("extension {kind}/{id}")));
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Workflows
    // ---------------------------------------------------------------------

    /// Insère ou met à jour un workflow.
    ///
    /// `created_at` et `last_run_at` sont préservés lors d'une mise à jour :
    /// seul [`Store::record_workflow_run`] fait avancer `last_run_at`.
    pub fn upsert_workflow(&self, wf: &Workflow) -> IhResult<()> {
        require_non_empty("identifiant de workflow", &wf.id)?;
        let steps = encode_json("étapes du workflow", &wf.steps)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO workflows
                (id, project_id, name, interval_minutes, enabled, steps, last_run_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (id) DO UPDATE SET project_id = excluded.project_id,
                                            name = excluded.name,
                                            interval_minutes = excluded.interval_minutes,
                                            enabled = excluded.enabled,
                                            steps = excluded.steps",
            params![
                wf.id,
                wf.project_id.as_str(),
                wf.name,
                i64::from(wf.interval_minutes),
                wf.enabled,
                steps,
                wf.last_run_at,
                wf.created_at,
            ],
        )
        .map_err(|err| db_err(format!("enregistrement du workflow {}", wf.id), err))?;
        Ok(())
    }

    /// Liste les workflows d'un projet, du plus récent au plus ancien.
    pub fn list_workflows(&self, project_id: &ProjectId) -> IhResult<Vec<Workflow>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, name, interval_minutes, enabled, steps, last_run_at,
                        created_at
                 FROM workflows
                 WHERE project_id = ?1
                 ORDER BY created_at DESC, rowid DESC",
            )
            .map_err(|err| db_err("préparation de la liste des workflows", err))?;
        let rows = stmt
            .query_map(params![project_id.as_str()], workflow_from_row)
            .map_err(|err| db_err(format!("lecture des workflows de {project_id}"), err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err(format!("lecture des workflows de {project_id}"), err))
    }

    /// Enregistre une exécution de workflow et fait avancer `last_run_at`.
    ///
    /// Cette API journalise une exécution achevée : `started_at` et
    /// `finished_at` portent le même instant. Un suivi en direct (run ouvert
    /// puis terminé) nécessiterait une API supplémentaire, volontairement non
    /// exposée ici.
    pub fn record_workflow_run(&self, run: &NewWorkflowRun) -> IhResult<WorkflowRun> {
        require_non_empty("statut d'exécution", &run.status)?;
        let now = Utc::now();
        let mut conn = self.lock()?;
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de l'enregistrement du run", err))?;
        let touched = tx
            .execute(
                "UPDATE workflows SET last_run_at = ?2 WHERE id = ?1",
                params![run.workflow_id, now],
            )
            .map_err(|err| db_err(format!("mise à jour du workflow {}", run.workflow_id), err))?;
        if touched == 0 {
            return Err(IhError::NotFound(format!("workflow {}", run.workflow_id)));
        }
        tx.execute(
            "INSERT INTO workflow_runs (workflow_id, status, detail, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![run.workflow_id, run.status, run.detail, now],
        )
        .map_err(|err| db_err(format!("insertion du run de {}", run.workflow_id), err))?;
        let id = tx.last_insert_rowid();
        tx.commit()
            .map_err(|err| db_err("validation de l'enregistrement du run", err))?;
        Ok(WorkflowRun {
            id,
            workflow_id: run.workflow_id.clone(),
            status: run.status.clone(),
            detail: run.detail.clone(),
            started_at: now,
            finished_at: Some(now),
        })
    }

    /// Liste les exécutions d'un workflow, de la plus récente à la plus
    /// ancienne.
    pub fn list_workflow_runs(&self, workflow_id: &str, limit: u32) -> IhResult<Vec<WorkflowRun>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, workflow_id, status, detail, started_at, finished_at
                 FROM workflow_runs
                 WHERE workflow_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|err| db_err("préparation de la liste des runs", err))?;
        let rows = stmt
            .query_map(
                params![workflow_id, i64::from(limit)],
                workflow_run_from_row,
            )
            .map_err(|err| db_err(format!("lecture des runs de {workflow_id}"), err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err(format!("lecture des runs de {workflow_id}"), err))
    }

    // ---------------------------------------------------------------------
    // Audit
    // ---------------------------------------------------------------------

    /// Ajoute une entrée d'audit (approbations et décisions automatiques).
    pub fn append_audit(&self, entry: &NewAuditEntry) -> IhResult<()> {
        validate_choice("décision d'audit", &entry.decision, &AUDIT_DECISIONS)?;
        require_non_empty("catégorie d'audit", &entry.category)?;
        require_non_empty("résumé d'audit", &entry.summary)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO audit (category, summary, detail, decision, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                entry.category,
                entry.summary,
                entry.detail,
                entry.decision,
                Utc::now(),
            ],
        )
        .map_err(|err| db_err("ajout d'une entrée d'audit", err))?;
        Ok(())
    }

    /// Liste les entrées d'audit, de la plus récente à la plus ancienne.
    pub fn list_audit(&self, limit: u32) -> IhResult<Vec<AuditEntry>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, category, summary, detail, decision, created_at
                 FROM audit
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .map_err(|err| db_err("préparation de la liste d'audit", err))?;
        let rows = stmt
            .query_map(params![i64::from(limit)], audit_from_row)
            .map_err(|err| db_err("lecture de l'audit", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture de l'audit", err))
    }
}

/// Applique les PRAGMA de connexion.
///
/// `foreign_keys` est une option par connexion : elle doit être réactivée à
/// chaque ouverture. WAL persiste dans le fichier mais est redemandé sans
/// risque (et retombe silencieusement sur `memory` pour une base éphémère).
fn configure(conn: &Connection) -> IhResult<()> {
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|err| db_err("configuration du busy_timeout SQLite", err))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|err| db_err("configuration des PRAGMA SQLite", err))?;
    Ok(())
}

fn validate_extension_kind(kind: &str) -> IhResult<()> {
    validate_choice("type d'extension", kind, &EXTENSION_KINDS)
}

// -------------------------------------------------------------------------
// Conversions ligne SQLite -> type public
// -------------------------------------------------------------------------

fn project_from_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: get_id(row, 0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        created_at: row.get(3)?,
        last_opened_at: row.get(4)?,
        trusted: row.get(5)?,
    })
}

fn thread_from_row(row: &Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: get_id(row, 0)?,
        project_id: get_id(row, 1)?,
        title: row.get(2)?,
        agent_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: get_id(row, 0)?,
        thread_id: get_id(row, 1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        metadata: decode_opt_json(row, 4, "métadonnées du message")?,
        created_at: row.get(5)?,
    })
}

fn session_from_row(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: get_id(row, 0)?,
        project_id: get_id(row, 1)?,
        agent_id: row.get(2)?,
        thread_id: get_opt_id(row, 3)?,
        state: get_agent_state(row, 4)?,
        exit_code: row.get(5)?,
        created_at: row.get(6)?,
        ended_at: row.get(7)?,
    })
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: get_id(row, 0)?,
        project_id: get_id(row, 1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        state: get_task_state(row, 4)?,
        assignee: row.get(5)?,
        depends_on: decode_json(row, 6, "dépendances de la tâche")?,
        role: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn memory_from_row(row: &Row<'_>) -> rusqlite::Result<MemoryEntry> {
    Ok(MemoryEntry {
        id: row.get(0)?,
        project_id: get_opt_id(row, 1)?,
        scope: row.get(2)?,
        key: row.get(3)?,
        content: row.get(4)?,
        tags: decode_json(row, 5, "tags de la mémoire")?,
        source: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn extension_from_row(row: &Row<'_>) -> rusqlite::Result<InstalledExtension> {
    Ok(InstalledExtension {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        version: row.get(3)?,
        source: row.get(4)?,
        manifest: decode_json(row, 5, "manifeste d'extension")?,
        sha256: row.get(6)?,
        enabled: row.get(7)?,
        installed_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn workflow_from_row(row: &Row<'_>) -> rusqlite::Result<Workflow> {
    Ok(Workflow {
        id: row.get(0)?,
        project_id: get_id(row, 1)?,
        name: row.get(2)?,
        interval_minutes: get_u32(row, 3, "intervalle du workflow")?,
        enabled: row.get(4)?,
        steps: decode_json(row, 5, "étapes du workflow")?,
        last_run_at: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn workflow_run_from_row(row: &Row<'_>) -> rusqlite::Result<WorkflowRun> {
    Ok(WorkflowRun {
        id: row.get(0)?,
        workflow_id: row.get(1)?,
        status: row.get(2)?,
        detail: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
    })
}

fn audit_from_row(row: &Row<'_>) -> rusqlite::Result<AuditEntry> {
    Ok(AuditEntry {
        id: row.get(0)?,
        category: row.get(1)?,
        summary: row.get(2)?,
        detail: row.get(3)?,
        decision: row.get(4)?,
        created_at: row.get(5)?,
    })
}
