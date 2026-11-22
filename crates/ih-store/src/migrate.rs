//! Migrations versionnées du schéma SQLite.
//!
//! # Mécanisme
//!
//! La version fait foi dans `PRAGMA user_version` (et non dans une table
//! `schema_migrations`, pour éviter un aller-retour supplémentaire à chaque
//! ouverture). Chaque migration est appliquée dans une transaction avec
//! `PRAGMA user_version` mis à jour dans la même transaction : en cas d'échec,
//! la base reste à la version précédente et peut être rouverte.
//!
//! - **v1** : schéma initial complet (projets, conversations, messages,
//!   sessions, tâches, mémoire, extensions, workflows, audit, réglages) ;
//! - **v2** : index de recherche mémoire et index de tri de l'audit.
//!
//! Une base dont `user_version` dépasse [`SCHEMA_VERSION`] est refusée plutôt
//! que lue avec un schéma supposé compatible.

use ih_core::{IhError, IhResult};
use rusqlite::Connection;

use crate::util::db_err;

/// Version de schéma attendue par ce crate.
pub const SCHEMA_VERSION: u32 = 2;

/// v1 — schéma initial.
///
/// Choix de modélisation notables :
/// - `tasks.depends_on` est un tableau JSON d'identifiants de tâches stocké en
///   texte : l'ensemble est toujours relu d'un bloc, jamais joint par SQL ;
/// - `memory.tags` est un tableau JSON en texte ;
/// - `extensions.manifest` et `workflows.steps` conservent le JSON d'origine ;
/// - `messages`, `threads`, `sessions`, `tasks`, `memory` et `workflows`
///   référencent leur projet avec `ON DELETE CASCADE` : supprimer un projet ne
///   supprime que les métadonnées IH qui lui appartiennent ;
/// - `sessions.thread_id` utilise `ON DELETE SET NULL` pour conserver
///   l'historique d'exécution même si la conversation est supprimée ;
/// - les horodatages sont du texte RFC 3339 en UTC (format produit par
///   `chrono`), ce qui reste triable lexicographiquement.
pub(crate) const V1: &str = r#"
CREATE TABLE projects (
    id             TEXT PRIMARY KEY NOT NULL,
    path           TEXT NOT NULL,
    name           TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    last_opened_at TEXT NOT NULL,
    trusted        INTEGER NOT NULL DEFAULT 0 CHECK (trusted IN (0, 1))
);

CREATE INDEX idx_projects_last_opened ON projects (last_opened_at DESC);

CREATE TABLE threads (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    agent_id   TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_threads_project ON threads (project_id, updated_at DESC);

CREATE TABLE messages (
    id         TEXT PRIMARY KEY NOT NULL,
    thread_id  TEXT NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    role       TEXT NOT NULL,
    content    TEXT NOT NULL,
    metadata   TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_messages_thread ON messages (thread_id, created_at);

CREATE TABLE sessions (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    agent_id   TEXT NOT NULL,
    thread_id  TEXT REFERENCES threads (id) ON DELETE SET NULL,
    prompt     TEXT NOT NULL,
    state      TEXT NOT NULL,
    exit_code  INTEGER,
    created_at TEXT NOT NULL,
    ended_at   TEXT
);

CREATE INDEX idx_sessions_project ON sessions (project_id, created_at DESC);

CREATE TABLE tasks (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    state       TEXT NOT NULL,
    assignee    TEXT,
    depends_on  TEXT NOT NULL DEFAULT '[]',
    role        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX idx_tasks_project ON tasks (project_id, created_at);

CREATE TABLE memory (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT REFERENCES projects (id) ON DELETE CASCADE,
    scope      TEXT NOT NULL,
    key        TEXT,
    content    TEXT NOT NULL,
    tags       TEXT NOT NULL DEFAULT '[]',
    source     TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_memory_project ON memory (project_id, updated_at DESC);

CREATE TABLE extensions (
    id           TEXT NOT NULL,
    kind         TEXT NOT NULL,
    name         TEXT NOT NULL,
    version      TEXT NOT NULL,
    source       TEXT NOT NULL,
    manifest     TEXT NOT NULL,
    sha256       TEXT,
    enabled      INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    installed_at TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (id, kind)
);

CREATE INDEX idx_extensions_kind ON extensions (kind, name);

CREATE TABLE workflows (
    id               TEXT PRIMARY KEY NOT NULL,
    project_id       TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    interval_minutes INTEGER NOT NULL CHECK (interval_minutes >= 0),
    enabled          INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    steps            TEXT NOT NULL DEFAULT '[]',
    last_run_at      TEXT,
    created_at       TEXT NOT NULL
);

CREATE INDEX idx_workflows_project ON workflows (project_id);

CREATE TABLE workflow_runs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id TEXT NOT NULL REFERENCES workflows (id) ON DELETE CASCADE,
    status      TEXT NOT NULL,
    detail      TEXT,
    started_at  TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX idx_workflow_runs_workflow ON workflow_runs (workflow_id, id DESC);

CREATE TABLE audit (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    category   TEXT NOT NULL,
    summary    TEXT NOT NULL,
    detail     TEXT,
    decision   TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

/// v2 — index ajoutés après la première mise en service.
///
/// `idx_memory_scope_updated` accélère la recherche filtrée par portée/projet
/// et le tri par fraîcheur ; `idx_audit_created_at` accélère l'horodatage des
/// entrées d'audit (la liste reste triée par `id` décroissant).
pub(crate) const V2: &str = r#"
CREATE INDEX IF NOT EXISTS idx_memory_scope_updated
    ON memory (scope, project_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_created_at
    ON audit (created_at DESC);
"#;

/// Applique les migrations manquantes et renvoie la version courante.
pub(crate) fn migrate(conn: &mut Connection) -> IhResult<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|err| db_err("lecture de PRAGMA user_version", err))?;
    let current = u32::try_from(current).map_err(|_| {
        IhError::Db(format!(
            "PRAGMA user_version invalide (négatif) : {current}"
        ))
    })?;

    if current > SCHEMA_VERSION {
        return Err(IhError::Db(format!(
            "base en version de schéma {current}, plus récente que la version supportée \
             {SCHEMA_VERSION} ; mettez l'application à jour"
        )));
    }

    if current < 1 {
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de la migration v1", err))?;
        tx.execute_batch(V1)
            .map_err(|err| db_err("application de la migration v1", err))?;
        tx.pragma_update(None, "user_version", 1_i64)
            .map_err(|err| db_err("écriture de user_version = 1", err))?;
        tx.commit()
            .map_err(|err| db_err("validation de la migration v1", err))?;
        tracing::info!(version = 1, "migration du schéma ih-store appliquée");
    }

    if current < 2 {
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de la migration v2", err))?;
        tx.execute_batch(V2)
            .map_err(|err| db_err("application de la migration v2", err))?;
        tx.pragma_update(None, "user_version", 2_i64)
            .map_err(|err| db_err("écriture de user_version = 2", err))?;
        tx.commit()
            .map_err(|err| db_err("validation de la migration v2", err))?;
        tracing::info!(version = 2, "migration du schéma ih-store appliquée");
    }

    Ok(())
}
