//! État partagé de l'application Tauri.

use crate::commands::workspace::PtyEntry;
use ih_agents::{AgentSupervisor, EventSink};
use ih_chat::Canceller;
use ih_core::config::AppConfig;
use ih_core::events::CoreEvent;
use ih_core::platform::PlatformInfo;
use ih_core::{AppPaths, IhError, IhResult, ResourcePolicy};
use ih_extensions::mcp::McpClient;
use ih_mcp::McpSession;
use ih_i18n::I18n;
use ih_security::approval::ApprovalRegistry;
use ih_security::secrets::{KeyringSecretStore, SecretStore, SecretStoreStatus};
use ih_store::Store;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter};

/// Modification préparée mais non appliquée (diff en attente de décision).
#[derive(Clone)]
pub struct PendingDiff {
    pub project_id: String,
    pub path: String,
    /// `None` pour un fichier nouveau (aucune empreinte de référence).
    pub before_sha256: Option<String>,
    pub content: String,
    /// Vrai si l'acceptation doit supprimer le fichier.
    pub delete: bool,
    /// `editor` ou `agent:<session_id>` : indique d'où vient la modification.
    pub origin: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateStatus {
    pub state: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub progress: u8,
    pub error: Option<String>,
}

pub struct AppState {
    pub paths: AppPaths,
    pub store: Arc<Store>,
    pub config: RwLock<AppConfig>,
    pub secrets: Arc<dyn SecretStore>,
    pub approvals: ApprovalRegistry,
    pub supervisor: Arc<AgentSupervisor>,
    pub pty: Mutex<HashMap<String, PtyEntry>>,
    pub mcp: Mutex<HashMap<String, McpClient>>,
    /// Sessions MCP distantes (Streamable HTTP / SSE), sérialisées par un
    /// mutex asynchrone : une session est retirée le temps d'un appel puis
    /// remise en place, ce qui évite tout accès concurrent.
    pub mcp_sessions: tokio::sync::Mutex<HashMap<String, McpSession>>,
    /// Tours de chat en cours, annulables par identifiant de requête.
    pub chat: Arc<Mutex<HashMap<String, Canceller>>>,
    pub i18n: RwLock<I18n>,
    pub platform: PlatformInfo,
    pub update_status: Mutex<UpdateStatus>,
    pub staged_update: Mutex<Option<ih_update::StagedUpdate>>,
    /// Modifications préparées, en attente d'acceptation ou de rejet.
    pub pending_diffs: Arc<Mutex<HashMap<String, PendingDiff>>>,
    /// Sessions dont les événements doivent être persistés (`ih://agent-event`).
    pub agent_sessions: Arc<Mutex<HashSet<String>>>,
    pub app_handle: AppHandle,
    /// Garde le writer non bloquant en vie : sans lui, les dernières lignes
    /// de journal ne sont jamais écrites.
    #[allow(dead_code)]
    pub log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl AppState {
    pub fn init(app_handle: AppHandle, paths: AppPaths) -> IhResult<Self> {
        paths.ensure()?;
        // Les lanceurs graphiques n'héritent pas du PATH de ~/.bashrc : sans
        // cela, un CLI installé dans ~/.opencode/bin serait « introuvable ».
        let added = ih_core::platform::augment_path_from_login_shell();
        if !added.is_empty() {
            tracing::info!(dirs = ?added, "PATH complété depuis le shell de connexion");
        }
        let config = AppConfig::load_or_create(&paths.config_file())?;
        let log_guard = crate::logging::init(&paths, &config.logging.level)?;
        let store = Arc::new(Store::open(&paths.db_path())?);
        let locale = ih_i18n::Locale::parse(&config.locale).unwrap_or_default();
        let policy: ResourcePolicy = config.resources.clone();

        // Sessions d'agent actives : leurs événements sont persistés et
        // diffusés sous une forme typée (`ih://agent-event`).
        let agent_sessions: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let sink_app = app_handle.clone();
        let sink_store = store.clone();
        let sink_sessions = agent_sessions.clone();
        let sink_seqs: Arc<Mutex<HashMap<String, u32>>> = Arc::new(Mutex::new(HashMap::new()));
        let sink: EventSink = Arc::new(move |event: CoreEvent| {
            let _ = sink_app.emit(event.name(), &event);
            let session_id = match &event {
                CoreEvent::AgentOutput { session_id, .. }
                | CoreEvent::AgentState { session_id, .. } => session_id.clone(),
                _ => return,
            };
            let is_agent = sink_sessions
                .lock()
                .map(|set| set.contains(&session_id))
                .unwrap_or(false);
            if !is_agent {
                return;
            }
            let (kind, payload) = match &event {
                CoreEvent::AgentOutput { stream, data, .. } => (
                    "output",
                    serde_json::json!({
                        "stream": match stream {
                            ih_core::events::OutputStream::Stdout => "stdout",
                            ih_core::events::OutputStream::Stderr => "stderr",
                        },
                        "data": data.chars().take(4096).collect::<String>(),
                    }),
                ),
                CoreEvent::AgentState { state, .. } => {
                    ("state", serde_json::json!({ "state": state }))
                }
                _ => return,
            };
            let seq = {
                let mut seqs = match sink_seqs.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                let entry = seqs.entry(session_id.clone()).or_insert(0);
                let value = *entry;
                *entry = entry.saturating_add(1);
                value
            };
            // Rétention bornée : on n'écrit pas indéfiniment dans la base.
            if seq < 4000 {
                if let Ok(sid) = ih_core::SessionId::parse(session_id.clone()) {
                    let _ = sink_store.append_agent_event(&ih_store::NewAgentEvent {
                        session_id: sid,
                        seq,
                        kind: kind.to_string(),
                        payload: payload.clone(),
                    });
                }
            }
            let _ = sink_app.emit(
                "ih://agent-event",
                serde_json::json!({
                    "session_id": session_id,
                    "seq": seq,
                    "kind": kind,
                    "payload": payload,
                }),
            );
        });

        let supervisor = Arc::new(AgentSupervisor::new(policy, sink)?);

        Ok(Self {
            paths,
            store,
            i18n: RwLock::new(I18n::new(locale)),
            config: RwLock::new(config),
            secrets: Arc::new(KeyringSecretStore::new("dev.ih.desktop")),
            approvals: ApprovalRegistry::default_ttl(),
            supervisor,
            pty: Mutex::new(HashMap::new()),
            mcp: Mutex::new(HashMap::new()),
            mcp_sessions: tokio::sync::Mutex::new(HashMap::new()),
            chat: Arc::new(Mutex::new(HashMap::new())),
            platform: PlatformInfo::detect(),
            update_status: Mutex::new(UpdateStatus {
                state: "idle".into(),
                ..Default::default()
            }),
            staged_update: Mutex::new(None),
            pending_diffs: Arc::new(Mutex::new(HashMap::new())),
            agent_sessions,
            app_handle,
            log_guard,
        })
    }

    pub fn config_snapshot(&self) -> IhResult<AppConfig> {
        self.config
            .read()
            .map(|c| c.clone())
            .map_err(|_| IhError::Internal("configuration verrouillée".into()))
    }

    pub fn update_config(&self, mutate: impl FnOnce(&mut AppConfig)) -> IhResult<AppConfig> {
        let mut guard = self
            .config
            .write()
            .map_err(|_| IhError::Internal("configuration verrouillée".into()))?;
        mutate(&mut guard);
        guard.validate()?;
        guard.save(&self.paths.config_file())?;
        Ok(guard.clone())
    }

    pub fn secret_status(&self) -> SecretStoreStatus {
        self.secrets.status()
    }

    pub fn set_locale(&self, locale: &str) -> IhResult<AppConfig> {
        let parsed = ih_i18n::Locale::parse(locale)
            .ok_or_else(|| IhError::Invalid(format!("langue non prise en charge : {locale}")))?;
        if let Ok(mut i18n) = self.i18n.write() {
            *i18n = I18n::new(parsed);
        }
        self.update_config(|config| config.locale = parsed.code().to_string())
    }
}
