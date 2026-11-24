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
    AgentEventRow, AgentSessionDetail, AuditEntry, InstalledExtension, McpCallRecord,
    McpServerRecord, McpToolRecord, MemoryEntry, Message, NewAgentEvent, NewAuditEntry, NewMcpCall,
    NewMemoryEntry, NewMessage, NewSession, NewTask, NewWorkflowRun, Project, SessionRow, Task,
    Thread, Workflow, WorkflowRun,
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

    /// Récupère une session par identifiant.
    pub fn get_session(&self, id: &SessionId) -> IhResult<Option<SessionRow>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, agent_id, thread_id, state, exit_code, created_at, ended_at
                 FROM sessions WHERE id = ?1",
            )
            .map_err(|err| db_err(format!("préparation de la lecture de {id}"), err))?;
        let mut rows = stmt
            .query_map(params![id.as_str()], session_from_row)
            .map_err(|err| db_err(format!("lecture de la session {id}"), err))?;
        match rows.next() {
            Some(row) => {
                Ok(Some(row.map_err(|err| {
                    db_err(format!("lecture de la session {id}"), err)
                })?))
            }
            None => Ok(None),
        }
    }

    /// Enregistre l'adaptateur, la tâche et le répertoire isolé d'une session.
    pub fn update_session_meta(
        &self,
        id: &SessionId,
        adapter: Option<&str>,
        task: Option<&str>,
        workdir: Option<&str>,
    ) -> IhResult<()> {
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE sessions
                 SET adapter = COALESCE(?2, adapter),
                     task = COALESCE(?3, task),
                     workdir = COALESCE(?4, workdir)
                 WHERE id = ?1",
                params![id.as_str(), adapter, task, workdir],
            )
            .map_err(|err| db_err(format!("mise à jour des métadonnées de {id}"), err))?;
        if updated == 0 {
            return Err(IhError::NotFound(format!("session {id}")));
        }
        Ok(())
    }

    /// Termine une session : état, code de sortie, résumé et fichiers touchés.
    pub fn complete_session(
        &self,
        id: &SessionId,
        state: AgentState,
        exit_code: Option<i32>,
        summary: Option<&str>,
        files: &[String],
    ) -> IhResult<()> {
        let files_json = encode_json("fichiers modifiés", files)?;
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE sessions
                 SET state = ?2, exit_code = ?3, ended_at = ?4, summary = ?5, files = ?6
                 WHERE id = ?1",
                params![
                    id.as_str(),
                    agent_state_str(state),
                    exit_code,
                    Utc::now(),
                    summary,
                    files_json
                ],
            )
            .map_err(|err| db_err(format!("fin de la session {id}"), err))?;
        if updated == 0 {
            return Err(IhError::NotFound(format!("session {id}")));
        }
        Ok(())
    }

    /// Session complète avec ses métadonnées, pour l'interface.
    pub fn get_session_detail(&self, id: &SessionId) -> IhResult<Option<AgentSessionDetail>> {
        let Some(session) = self.get_session(id)? else {
            return Ok(None);
        };
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare("SELECT adapter, task, workdir, summary, files FROM sessions WHERE id = ?1")
            .map_err(|err| db_err(format!("préparation de la session {id}"), err))?;
        let detail = stmt
            .query_row(params![id.as_str()], |row| {
                let adapter: Option<String> = row.get(0)?;
                let task: Option<String> = row.get(1)?;
                let workdir: Option<String> = row.get(2)?;
                let summary: Option<String> = row.get(3)?;
                let files: Option<String> = row.get(4)?;
                Ok((adapter, task, workdir, summary, files))
            })
            .map_err(|err| db_err(format!("lecture des métadonnées de {id}"), err))?;
        let files: Vec<String> = match detail.4 {
            Some(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(Some(AgentSessionDetail {
            session,
            adapter: detail.0,
            task: detail.1,
            workdir: detail.2,
            summary: detail.3,
            files,
        }))
    }

    /// Ajoute un événement de session (numéro de séquence fourni par l'appelant).
    pub fn append_agent_event(&self, new: &NewAgentEvent) -> IhResult<AgentEventRow> {
        if new.kind.trim().is_empty() {
            return Err(IhError::Invalid("type d'événement vide".into()));
        }
        let payload = encode_json("événement d'agent", &new.payload)?;
        let now = Utc::now();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO agent_events (session_id, seq, kind, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                new.session_id.as_str(),
                i64::from(new.seq),
                new.kind,
                payload,
                now
            ],
        )
        .map_err(|err| db_err(format!("ajout d'un événement à {}", new.session_id), err))?;
        let id = conn.last_insert_rowid();
        Ok(AgentEventRow {
            id,
            session_id: new.session_id.clone(),
            seq: new.seq,
            kind: new.kind.clone(),
            payload: new.payload.clone(),
            created_at: now,
        })
    }

    /// Événements d'une session, dans l'ordre, bornés par `limit`.
    pub fn list_agent_events(
        &self,
        session_id: &SessionId,
        limit: u32,
    ) -> IhResult<Vec<AgentEventRow>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, seq, kind, payload, created_at
                 FROM agent_events WHERE session_id = ?1
                 ORDER BY seq ASC LIMIT ?2",
            )
            .map_err(|err| db_err("préparation de la liste d'événements", err))?;
        let rows = stmt
            .query_map(params![session_id.as_str(), i64::from(limit)], |row| {
                let raw: String = row.get(4)?;
                Ok(AgentEventRow {
                    id: row.get(0)?,
                    session_id: SessionId::parse(row.get::<_, String>(1)?)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    seq: row.get::<_, i64>(2)? as u32,
                    kind: row.get(3)?,
                    payload: serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null),
                    created_at: row.get(5)?,
                })
            })
            .map_err(|err| db_err(format!("lecture des événements de {session_id}"), err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err(format!("lecture des événements de {session_id}"), err))
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

    // ---------------------------------------------------------------------
    // MCP (serveurs distants, catalogue, appels)
    // ---------------------------------------------------------------------

    /// Liste les serveurs MCP, du plus récent au plus ancien.
    pub fn list_mcp_servers(&self) -> IhResult<Vec<McpServerRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, url, transport, headers, timeout_secs, enabled,
                        allow_auto_approve, last_status, last_error, last_connected_at,
                        created_at, updated_at
                 FROM mcp_servers
                 ORDER BY created_at DESC, rowid DESC",
            )
            .map_err(|err| db_err("préparation de la liste des serveurs MCP", err))?;
        let rows = stmt
            .query_map([], mcp_server_from_row)
            .map_err(|err| db_err("lecture des serveurs MCP", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture des serveurs MCP", err))
    }

    /// Lit un serveur MCP par identifiant.
    pub fn get_mcp_server(&self, id: &str) -> IhResult<Option<McpServerRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, url, transport, headers, timeout_secs, enabled,
                        allow_auto_approve, last_status, last_error, last_connected_at,
                        created_at, updated_at
                 FROM mcp_servers
                 WHERE id = ?1",
            )
            .map_err(|err| db_err("préparation de la lecture d'un serveur MCP", err))?;
        let mut rows = stmt
            .query_map(params![id], mcp_server_from_row)
            .map_err(|err| db_err(format!("lecture du serveur MCP {id}"), err))?;
        rows.next()
            .transpose()
            .map_err(|err| db_err(format!("lecture du serveur MCP {id}"), err))
    }

    /// Insère ou met à jour un serveur MCP. `created_at` est préservé lors
    /// d'une mise à jour.
    pub fn upsert_mcp_server(&self, server: &McpServerRecord) -> IhResult<()> {
        require_non_empty("identifiant de serveur MCP", &server.id)?;
        require_non_empty("nom de serveur MCP", &server.name)?;
        require_non_empty("URL de serveur MCP", &server.url)?;
        validate_choice(
            "transport MCP",
            &server.transport,
            &["streamable_http", "sse"],
        )?;
        if server.timeout_secs == 0 {
            return Err(IhError::Invalid(
                "le délai d'un serveur MCP doit être supérieur à zéro".into(),
            ));
        }
        let headers = encode_json("en-têtes du serveur MCP", &server.headers)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO mcp_servers
                (id, name, url, transport, headers, timeout_secs, enabled,
                 allow_auto_approve, last_status, last_error, last_connected_at,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (id) DO UPDATE SET
                 name = excluded.name,
                 url = excluded.url,
                 transport = excluded.transport,
                 headers = excluded.headers,
                 timeout_secs = excluded.timeout_secs,
                 enabled = excluded.enabled,
                 allow_auto_approve = excluded.allow_auto_approve,
                 last_status = excluded.last_status,
                 last_error = excluded.last_error,
                 last_connected_at = excluded.last_connected_at,
                 updated_at = excluded.updated_at",
            params![
                server.id,
                server.name,
                server.url,
                server.transport,
                headers,
                i64::from(server.timeout_secs),
                server.enabled,
                server.allow_auto_approve,
                server.last_status,
                server.last_error,
                server.last_connected_at,
                server.created_at,
                server.updated_at,
            ],
        )
        .map_err(|err| db_err(format!("enregistrement du serveur MCP {}", server.id), err))?;
        Ok(())
    }

    /// Met à jour l'état de connexion d'un serveur MCP.
    pub fn set_mcp_server_status(
        &self,
        id: &str,
        status: &str,
        last_error: Option<&str>,
        last_connected_at: Option<DateTime<Utc>>,
    ) -> IhResult<()> {
        validate_choice(
            "état de connexion MCP",
            status,
            &["disconnected", "connecting", "connected", "error"],
        )?;
        let now = Utc::now();
        let conn = self.lock()?;
        let touched = conn
            .execute(
                "UPDATE mcp_servers
                 SET last_status = ?2, last_error = ?3,
                     last_connected_at = COALESCE(?4, last_connected_at),
                     updated_at = ?5
                 WHERE id = ?1",
                params![id, status, last_error, last_connected_at, now],
            )
            .map_err(|err| db_err(format!("mise à jour de l'état du serveur MCP {id}"), err))?;
        if touched == 0 {
            return Err(IhError::NotFound(format!("serveur MCP {id}")));
        }
        Ok(())
    }

    /// Supprime un serveur MCP, son catalogue et son historique d'appels.
    pub fn delete_mcp_server(&self, id: &str) -> IhResult<()> {
        let conn = self.lock()?;
        let touched = conn
            .execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])
            .map_err(|err| db_err(format!("suppression du serveur MCP {id}"), err))?;
        if touched == 0 {
            return Err(IhError::NotFound(format!("serveur MCP {id}")));
        }
        Ok(())
    }

    /// Met à jour le catalogue d'outils d'un serveur.
    ///
    /// Les outils absents du nouveau catalogue sont supprimés ; l'état activé
    /// (choix explicite de l'utilisateur) est préservé par nom pour les outils
    /// qui existaient déjà, sinon `enabled` vaut faux.
    pub fn refresh_mcp_tools(&self, server_id: &str, tools: &[McpToolRecord]) -> IhResult<()> {
        let mut conn = self.lock()?;
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début du rafraîchissement du catalogue MCP", err))?;

        // 1) relevé des activations existantes AVANT de remplacer le catalogue.
        let mut previous: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        {
            let mut stmt = tx
                .prepare("SELECT name, enabled FROM mcp_tools WHERE server_id = ?1")
                .map_err(|err| db_err("préparation du relevé des outils MCP", err))?;
            let rows = stmt
                .query_map(params![server_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                })
                .map_err(|err| db_err("relevé des outils MCP existants", err))?;
            for entry in rows {
                let (name, enabled) =
                    entry.map_err(|err| db_err("relevé des outils MCP existants", err))?;
                previous.insert(name, enabled);
            }
        }

        // 2) remplacement transactionnel du catalogue.
        tx.execute(
            "DELETE FROM mcp_tools WHERE server_id = ?1",
            params![server_id],
        )
        .map_err(|err| db_err("purge du catalogue MCP", err))?;
        for tool in tools {
            let input_schema = encode_json("schéma d'entrée MCP", &tool.input_schema)?;
            let output_schema = match &tool.output_schema {
                Some(schema) => Some(encode_json("schéma de sortie MCP", schema)?),
                None => None,
            };
            let enabled = previous.get(&tool.name).copied().unwrap_or(false);
            tx.execute(
                "INSERT INTO mcp_tools
                    (server_id, name, description, input_schema, output_schema, category,
                     risk, enabled, discovered_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    server_id,
                    tool.name,
                    tool.description,
                    input_schema,
                    output_schema,
                    tool.category,
                    tool.risk,
                    enabled,
                    tool.discovered_at,
                ],
            )
            .map_err(|err| db_err(format!("enregistrement de l'outil MCP {}", tool.name), err))?;
        }
        tx.commit()
            .map_err(|err| db_err("validation du catalogue MCP", err))?;
        Ok(())
    }

    /// Liste les outils MCP (tous serveurs, ou un seul).
    pub fn list_mcp_tools(&self, server_id: Option<&str>) -> IhResult<Vec<McpToolRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT server_id, name, description, input_schema, output_schema,
                        category, risk, enabled, discovered_at
                 FROM mcp_tools
                 WHERE (?1 IS NULL OR server_id = ?1)
                 ORDER BY server_id, name",
            )
            .map_err(|err| db_err("préparation de la liste des outils MCP", err))?;
        let rows = stmt
            .query_map(params![server_id], mcp_tool_from_row)
            .map_err(|err| db_err("lecture des outils MCP", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture des outils MCP", err))
    }

    /// Lit un outil MCP précis.
    pub fn get_mcp_tool(&self, server_id: &str, name: &str) -> IhResult<Option<McpToolRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT server_id, name, description, input_schema, output_schema,
                        category, risk, enabled, discovered_at
                 FROM mcp_tools
                 WHERE server_id = ?1 AND name = ?2",
            )
            .map_err(|err| db_err("préparation de la lecture d'un outil MCP", err))?;
        let mut rows = stmt
            .query_map(params![server_id, name], mcp_tool_from_row)
            .map_err(|err| db_err(format!("lecture de l'outil MCP {name}"), err))?;
        rows.next()
            .transpose()
            .map_err(|err| db_err(format!("lecture de l'outil MCP {name}"), err))
    }

    /// Active ou désactive un outil MCP (jamais l'inverse : c'est un refus).
    pub fn set_mcp_tool_enabled(&self, server_id: &str, name: &str, enabled: bool) -> IhResult<()> {
        let conn = self.lock()?;
        let touched = conn
            .execute(
                "UPDATE mcp_tools SET enabled = ?3 WHERE server_id = ?1 AND name = ?2",
                params![server_id, name, enabled],
            )
            .map_err(|err| db_err(format!("mise à jour de l'outil MCP {name}"), err))?;
        if touched == 0 {
            return Err(IhError::NotFound(format!("outil MCP {server_id}/{name}")));
        }
        Ok(())
    }

    /// Enregistre un appel MCP terminé (ou refusé).
    pub fn record_mcp_call(&self, call: &NewMcpCall) -> IhResult<McpCallRecord> {
        require_non_empty("outil d'appel MCP", &call.tool_name)?;
        validate_choice(
            "appelant MCP",
            &call.caller,
            &["ui", "chat", "agent", "team", "automation"],
        )?;
        validate_choice(
            "statut d'appel MCP",
            &call.status,
            &["completed", "failed", "denied", "approval_required"],
        )?;
        let arguments = encode_json("arguments d'appel MCP", &call.arguments)?;
        let now = Utc::now();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO mcp_calls
                (server_id, tool_name, caller, caller_id, arguments, status, error,
                 duration_ms, started_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                call.server_id,
                call.tool_name,
                call.caller,
                call.caller_id,
                arguments,
                call.status,
                call.error,
                call.duration_ms
                    .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                now,
            ],
        )
        .map_err(|err| {
            db_err(
                format!("enregistrement de l'appel MCP {}", call.tool_name),
                err,
            )
        })?;
        let id = conn.last_insert_rowid();
        Ok(McpCallRecord {
            id,
            server_id: call.server_id.clone(),
            tool_name: call.tool_name.clone(),
            caller: call.caller.clone(),
            caller_id: call.caller_id.clone(),
            arguments: call.arguments.clone(),
            status: call.status.clone(),
            error: call.error.clone(),
            duration_ms: call.duration_ms,
            started_at: now,
            finished_at: Some(now),
        })
    }

    /// Liste les appels MCP, du plus récent au plus ancien.
    pub fn list_mcp_calls(
        &self,
        limit: u32,
        server_id: Option<&str>,
    ) -> IhResult<Vec<McpCallRecord>> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, server_id, tool_name, caller, caller_id, arguments, status, error,
                        duration_ms, started_at, finished_at
                 FROM mcp_calls
                 WHERE (?1 IS NULL OR server_id = ?1)
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|err| db_err("préparation de la liste des appels MCP", err))?;
        let rows = stmt
            .query_map(params![server_id, i64::from(limit)], mcp_call_from_row)
            .map_err(|err| db_err("lecture des appels MCP", err))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| db_err("lecture des appels MCP", err))
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

fn mcp_server_from_row(row: &Row<'_>) -> rusqlite::Result<McpServerRecord> {
    Ok(McpServerRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        url: row.get(2)?,
        transport: row.get(3)?,
        headers: decode_json(row, 4, "en-têtes du serveur MCP")?,
        timeout_secs: get_u32(row, 5, "délai du serveur MCP")?,
        enabled: row.get(6)?,
        allow_auto_approve: row.get(7)?,
        last_status: row.get(8)?,
        last_error: row.get(9)?,
        last_connected_at: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn mcp_tool_from_row(row: &Row<'_>) -> rusqlite::Result<McpToolRecord> {
    Ok(McpToolRecord {
        server_id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        input_schema: decode_json(row, 3, "schéma d'entrée MCP")?,
        output_schema: match row.get::<_, Option<String>>(4)? {
            Some(_) => Some(decode_json(row, 4, "schéma de sortie MCP")?),
            None => None,
        },
        category: row.get(5)?,
        risk: row.get(6)?,
        enabled: row.get(7)?,
        discovered_at: row.get(8)?,
    })
}

fn mcp_call_from_row(row: &Row<'_>) -> rusqlite::Result<McpCallRecord> {
    Ok(McpCallRecord {
        id: row.get(0)?,
        server_id: row.get(1)?,
        tool_name: row.get(2)?,
        caller: row.get(3)?,
        caller_id: row.get(4)?,
        arguments: decode_json(row, 5, "arguments d'appel MCP")?,
        status: row.get(6)?,
        error: row.get(7)?,
        duration_ms: row
            .get::<_, Option<i64>>(8)?
            .map(|value| value.max(0) as u64),
        started_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}
