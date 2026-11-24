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
pub const SCHEMA_VERSION: u32 = 6;

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

/// v3 — sessions d'agent : métadonnées d'exécution et flux d'événements.
///
/// `sessions` reçoit la tâche, le répertoire de travail isolé, l'adaptateur,
/// le résumé final et la liste JSON des fichiers touchés. `agent_events`
/// conserve les événements bornés d'une session pour la reprise après
/// redémarrage.
pub(crate) const V3: &str = r#"
ALTER TABLE sessions ADD COLUMN adapter TEXT;
ALTER TABLE sessions ADD COLUMN task TEXT;
ALTER TABLE sessions ADD COLUMN workdir TEXT;
ALTER TABLE sessions ADD COLUMN summary TEXT;
ALTER TABLE sessions ADD COLUMN files TEXT;

CREATE TABLE agent_events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_agent_events_session ON agent_events (session_id, seq);
"#;

/// v4 — intégration MCP native (serveurs distants, catalogue, appels).
///
/// Les jetons et secrets ne sont **jamais** stockés ici : seule la
/// configuration non secrète vit en base, le secret est lu dans le trousseau.
/// `mcp_tools.enabled` vaut 0 par défaut : un outil découvert n'est jamais
/// autorisé sans activation explicite. Les appels sont historisés pour
/// l'observabilité (serveur, outil, appelant, statut, durée, erreur), avec
/// cascade à la suppression du serveur.
pub(crate) const V4: &str = r#"
CREATE TABLE mcp_servers (
    id                 TEXT PRIMARY KEY NOT NULL,
    name               TEXT NOT NULL,
    url                TEXT NOT NULL,
    transport          TEXT NOT NULL CHECK (transport IN ('streamable_http', 'sse')),
    headers            TEXT NOT NULL DEFAULT '{}',
    timeout_secs       INTEGER NOT NULL DEFAULT 30 CHECK (timeout_secs > 0),
    enabled            INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    allow_auto_approve INTEGER NOT NULL DEFAULT 0 CHECK (allow_auto_approve IN (0, 1)),
    last_status        TEXT NOT NULL DEFAULT 'disconnected',
    last_error         TEXT,
    last_connected_at  TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

CREATE INDEX idx_mcp_servers_enabled ON mcp_servers (enabled);

CREATE TABLE mcp_tools (
    server_id     TEXT NOT NULL REFERENCES mcp_servers (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    input_schema  TEXT NOT NULL DEFAULT '{}',
    output_schema TEXT,
    category      TEXT NOT NULL DEFAULT 'write'
                  CHECK (category IN ('read', 'write', 'execute', 'destructive', 'network')),
    risk          TEXT NOT NULL DEFAULT 'medium'
                  CHECK (risk IN ('low', 'medium', 'high', 'critical')),
    enabled       INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    discovered_at TEXT NOT NULL,
    PRIMARY KEY (server_id, name)
);

CREATE INDEX idx_mcp_tools_server ON mcp_tools (server_id);

CREATE TABLE mcp_calls (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    server_id   TEXT NOT NULL REFERENCES mcp_servers (id) ON DELETE CASCADE,
    tool_name   TEXT NOT NULL,
    caller      TEXT NOT NULL,
    caller_id   TEXT,
    arguments   TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    duration_ms INTEGER,
    started_at  TEXT NOT NULL,
    finished_at TEXT
);

CREATE INDEX idx_mcp_calls_started ON mcp_calls (started_at DESC);
CREATE INDEX idx_mcp_calls_server ON mcp_calls (server_id, id DESC);
"#;

/// v5 — MCP : mémoriser l'existence d'un jeton sans jamais lire le trousseau.
///
/// `mcp_servers.has_secret` est un simple drapeau, comme `providers.has_secret`
/// dans la configuration : il permet d'afficher « jeton configuré » sans
/// solliciter le trousseau (qui déclencherait une demande de déverrouillage).
pub(crate) const V5: &str = r#"
ALTER TABLE mcp_servers ADD COLUMN has_secret INTEGER NOT NULL DEFAULT 0
    CHECK (has_secret IN (0, 1));
"#;

/// v6 — chat : appels d'outils MCP proposés par un modèle.
///
/// Chaque ligne est une **proposition** (arguments redactés pour l'audit) :
/// la décision humaine et l'exécution sont tracées dans les colonnes de
/// statut. Un redémarrage ne rejoue jamais une proposition restée `pending`.
pub(crate) const V6: &str = r#"
CREATE TABLE chat_tool_calls (
    id                 TEXT PRIMARY KEY NOT NULL,
    thread_id          TEXT NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    project_id         TEXT NOT NULL,
    request_id         TEXT NOT NULL,
    round              INTEGER NOT NULL DEFAULT 0,
    provider_call_id   TEXT NOT NULL,
    exposed_name       TEXT NOT NULL,
    server_id          TEXT NOT NULL,
    tool               TEXT NOT NULL,
    risk               TEXT NOT NULL,
    category           TEXT NOT NULL,
    fingerprint        TEXT NOT NULL,
    arguments_redacted TEXT NOT NULL,
    arguments_raw      TEXT NOT NULL,
    status             TEXT NOT NULL CHECK (status IN
        ('pending', 'denied', 'completed', 'failed', 'invalid', 'expired')),
    error              TEXT,
    result_text        TEXT,
    approval_id        TEXT,
    created_at         TEXT NOT NULL,
    decided_at         TEXT,
    finished_at        TEXT
);

CREATE INDEX idx_chat_tool_calls_thread ON chat_tool_calls (thread_id, created_at);
CREATE INDEX idx_chat_tool_calls_request ON chat_tool_calls (request_id, status);
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

    if current < 3 {
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de la migration v3", err))?;
        tx.execute_batch(V3)
            .map_err(|err| db_err("application de la migration v3", err))?;
        tx.pragma_update(None, "user_version", 3_i64)
            .map_err(|err| db_err("écriture de user_version = 3", err))?;
        tx.commit()
            .map_err(|err| db_err("validation de la migration v3", err))?;
        tracing::info!(version = 3, "migration du schéma ih-store appliquée");
    }

    if current < 4 {
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de la migration v4", err))?;
        tx.execute_batch(V4)
            .map_err(|err| db_err("application de la migration v4", err))?;
        tx.pragma_update(None, "user_version", 4_i64)
            .map_err(|err| db_err("écriture de user_version = 4", err))?;
        tx.commit()
            .map_err(|err| db_err("validation de la migration v4", err))?;
        tracing::info!(version = 4, "migration du schéma ih-store appliquée");
    }

    if current < 5 {
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de la migration v5", err))?;
        tx.execute_batch(V5)
            .map_err(|err| db_err("application de la migration v5", err))?;
        tx.pragma_update(None, "user_version", 5_i64)
            .map_err(|err| db_err("écriture de user_version = 5", err))?;
        tx.commit()
            .map_err(|err| db_err("validation de la migration v5", err))?;
        tracing::info!(version = 5, "migration du schéma ih-store appliquée");
    }

    if current < 6 {
        let tx = conn
            .transaction()
            .map_err(|err| db_err("début de la migration v6", err))?;
        tx.execute_batch(V6)
            .map_err(|err| db_err("application de la migration v6", err))?;
        tx.pragma_update(None, "user_version", 6_i64)
            .map_err(|err| db_err("écriture de user_version = 6", err))?;
        tx.commit()
            .map_err(|err| db_err("validation de la migration v6", err))?;
        tracing::info!(version = 6, "migration du schéma ih-store appliquée");
    }

    Ok(())
}
