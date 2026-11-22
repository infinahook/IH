//! État partagé de l'application Tauri.

use crate::commands::workspace::PtyEntry;
use ih_agents::{AgentSupervisor, EventSink};
use ih_chat::Canceller;
use ih_core::config::AppConfig;
use ih_core::events::CoreEvent;
use ih_core::platform::PlatformInfo;
use ih_core::{AppPaths, IhError, IhResult, ResourcePolicy};
use ih_extensions::mcp::McpClient;
use ih_i18n::I18n;
use ih_security::approval::ApprovalRegistry;
use ih_security::secrets::{KeyringSecretStore, SecretStore, SecretStoreStatus};
use ih_store::Store;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter};

/// Modification préparée mais non appliquée (diff en attente de décision).
pub struct PendingDiff {
    pub project_id: String,
    pub path: String,
    pub before_sha256: String,
    pub content: String,
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
    pub supervisor: AgentSupervisor,
    pub pty: Mutex<HashMap<String, PtyEntry>>,
    pub mcp: Mutex<HashMap<String, McpClient>>,
    /// Tours de chat en cours, annulables par identifiant de requête.
    pub chat: Arc<Mutex<HashMap<String, Canceller>>>,
    pub i18n: RwLock<I18n>,
    pub platform: PlatformInfo,
    pub update_status: Mutex<UpdateStatus>,
    pub staged_update: Mutex<Option<ih_update::StagedUpdate>>,
    /// Modifications préparées, en attente d'acceptation ou de rejet.
    pub pending_diffs: Mutex<HashMap<String, PendingDiff>>,
    pub app_handle: AppHandle,
    /// Garde le writer non bloquant en vie : sans lui, les dernières lignes
    /// de journal ne sont jamais écrites.
    #[allow(dead_code)]
    pub log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl AppState {
    pub fn init(app_handle: AppHandle, paths: AppPaths) -> IhResult<Self> {
        paths.ensure()?;
        let config = AppConfig::load_or_create(&paths.config_file())?;
        let log_guard = crate::logging::init(&paths, &config.logging.level)?;
        let store = Arc::new(Store::open(&paths.db_path())?);
        let locale = ih_i18n::Locale::parse(&config.locale).unwrap_or_default();
        let policy: ResourcePolicy = config.resources.clone();

        let sink_app = app_handle.clone();
        let sink: EventSink = Arc::new(move |event: CoreEvent| {
            let _ = sink_app.emit(event.name(), &event);
        });

        let supervisor = AgentSupervisor::new(policy, sink)?;

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
            chat: Arc::new(Mutex::new(HashMap::new())),
            platform: PlatformInfo::detect(),
            update_status: Mutex::new(UpdateStatus {
                state: "idle".into(),
                ..Default::default()
            }),
            staged_update: Mutex::new(None),
            pending_diffs: Mutex::new(HashMap::new()),
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
