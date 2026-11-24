//! Configuration applicative persistée en TOML.
//!
//! Règle absolue : **aucun secret dans ce fichier**. Les clés d'API vivent dans
//! le trousseau du système (voir `ih-security`) et la configuration ne conserve
//! qu'un booléen `has_secret` pour l'affichage.

use crate::error::{IhError, IhResult};
use crate::resources::ResourcePolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const SUPPORTED_LOCALES: [&str; 2] = ["fr", "en"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenaiCompatible,
    Anthropic,
    Google,
    Ollama,
}

impl ProviderKind {
    pub fn requires_secret(self) -> bool {
        !matches!(self, Self::Ollama)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    GeminiCli,
    Opencode,
    /// `pi` — assistant de code avec outils read/bash/edit/write.
    Pi,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelEntry {
    pub id: String,
    pub label: String,
    pub context_window: Option<u32>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub enabled: bool,
    pub default_model: Option<String>,
    pub models: Vec<ModelEntry>,
    /// Un secret est présent dans le trousseau — jamais le secret lui-même.
    pub has_secret: bool,
    pub timeout_secs: u64,
    /// Le fournisseur annonce le tool calling natif (capacité explicite,
    /// jamais devinée). Désactivé par défaut : le chat garde son
    /// comportement classique et le protocole texte reste disponible.
    pub supports_tools: bool,
    /// Chemin complet de l'endpoint de chat, relatif à l'origine de `base_url`
    /// (ex. `/v1/chat/completions`). Absent : chemin standard du protocole.
    pub endpoint_path: Option<String>,
    /// En-têtes HTTP additionnels (métadonnées). Jamais de secret ici.
    pub headers: std::collections::BTreeMap<String, String>,
    /// Streaming activé (SSE/NDJSON) ; sinon réponse complète en un bloc.
    pub streaming: bool,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Provider ajouté par l'utilisateur (non intégré).
    pub custom: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: ProviderKind::OpenaiCompatible,
            label: String::new(),
            base_url: String::new(),
            enabled: false,
            default_model: None,
            models: Vec::new(),
            has_secret: false,
            timeout_secs: 120,
            supports_tools: false,
            endpoint_path: None,
            headers: std::collections::BTreeMap::new(),
            streaming: true,
            temperature: None,
            max_tokens: None,
            custom: false,
            created_at: None,
            updated_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub id: String,
    pub kind: AgentKind,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub enabled: bool,
    pub env: BTreeMap<String, String>,
    /// Délai maximal d'exécution d'une session (secondes).
    pub timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: AgentKind::Custom,
            label: String::new(),
            program: String::new(),
            args: Vec::new(),
            enabled: false,
            env: BTreeMap::new(),
            timeout_secs: 1_800,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub channel: String,
    /// URL d'un manifeste signé (`manifest.json`).
    pub endpoint: Option<String>,
    pub check_interval_minutes: u32,
    pub auto_download: bool,
    /// Clé publique Ed25519 (hex) intégrée à la compilation en production.
    pub public_key_hex: Option<String>,
    pub allow_prerelease: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            channel: "stable".into(),
            endpoint: None,
            check_interval_minutes: 360,
            auto_download: false,
            public_key_hex: None,
            allow_prerelease: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub shell: String,
    pub args: Vec<String>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        Self {
            shell,
            args: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub sidebar_collapsed: bool,
    pub editor_font_size: u8,
    pub reduce_motion: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_collapsed: false,
            editor_font_size: 13,
            reduce_motion: false,
        }
    }
}

/// Réglages du workspace et du contexte projet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    /// Motifs d'exclusion additionnels (en plus de ceux d'`ih-context`).
    pub exclusions: Vec<String>,
    /// Nombre maximal de fichiers envoyés comme contexte.
    pub max_context_files: usize,
    /// Octets maximaux lus par fichier de contexte.
    pub max_context_bytes_per_file: usize,
    /// Octets maximaux pour l'ensemble du contexte.
    pub max_context_bytes: usize,
    /// Taille maximale d'un fichier ouvert dans l'éditeur.
    pub max_editor_bytes: usize,
    /// Restaurer les onglets ouverts au redémarrage.
    pub restore_tabs: bool,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            exclusions: Vec::new(),
            max_context_files: 12,
            max_context_bytes_per_file: 64 * 1024,
            max_context_bytes: 256 * 1024,
            max_editor_bytes: 2 * 1024 * 1024,
            restore_tabs: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: String,
    pub retention_days: u32,
    pub json: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            retention_days: 14,
            json: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema: u32,
    pub locale: String,
    pub theme: String,
    pub resources: ResourcePolicy,
    pub providers: Vec<ProviderConfig>,
    pub agents: Vec<AgentConfig>,
    pub updates: UpdateConfig,
    pub terminal: TerminalConfig,
    pub ui: UiConfig,
    pub logging: LoggingConfig,
    pub workspace: WorkspaceConfig,
    pub recent_projects: Vec<String>,
    pub last_project: Option<String>,
    pub telemetry: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            locale: "fr".into(),
            theme: "dark".into(),
            resources: ResourcePolicy::default(),
            providers: default_providers(),
            agents: default_agents(),
            updates: UpdateConfig::default(),
            terminal: TerminalConfig::default(),
            ui: UiConfig::default(),
            logging: LoggingConfig::default(),
            workspace: WorkspaceConfig::default(),
            recent_projects: Vec::new(),
            last_project: None,
            telemetry: false,
        }
    }
}

/// Fournisseurs déclarés au premier lancement. Aucun n'est actif sans clé.
fn default_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: "openai".into(),
            kind: ProviderKind::OpenaiCompatible,
            label: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            enabled: true,
            has_secret: false,
            ..Default::default()
        },
        ProviderConfig {
            id: "anthropic".into(),
            kind: ProviderKind::Anthropic,
            label: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            enabled: true,
            has_secret: false,
            ..Default::default()
        },
        ProviderConfig {
            id: "google".into(),
            kind: ProviderKind::Google,
            label: "Google Gemini".into(),
            base_url: "https://generativelanguage.googleapis.com".into(),
            enabled: true,
            has_secret: false,
            ..Default::default()
        },
        ProviderConfig {
            id: "openrouter".into(),
            kind: ProviderKind::OpenaiCompatible,
            label: "OpenRouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            enabled: true,
            has_secret: false,
            ..Default::default()
        },
        ProviderConfig {
            id: "ollama".into(),
            kind: ProviderKind::Ollama,
            label: "Ollama (local)".into(),
            base_url: "http://127.0.0.1:11434".into(),
            enabled: true,
            has_secret: false,
            ..Default::default()
        },
    ]
}

fn default_agents() -> Vec<AgentConfig> {
    vec![
        AgentConfig {
            id: "claude-code".into(),
            kind: AgentKind::ClaudeCode,
            label: "Claude Code".into(),
            program: "claude".into(),
            enabled: true,
            ..Default::default()
        },
        AgentConfig {
            id: "codex".into(),
            kind: AgentKind::Codex,
            label: "Codex".into(),
            program: "codex".into(),
            enabled: true,
            ..Default::default()
        },
        AgentConfig {
            id: "gemini-cli".into(),
            kind: AgentKind::GeminiCli,
            label: "Gemini CLI".into(),
            program: "gemini".into(),
            enabled: true,
            ..Default::default()
        },
        AgentConfig {
            id: "opencode".into(),
            kind: AgentKind::Opencode,
            label: "OpenCode".into(),
            program: "opencode".into(),
            enabled: true,
            ..Default::default()
        },
        AgentConfig {
            id: "pi".into(),
            kind: AgentKind::Pi,
            label: "pi".into(),
            program: "pi".into(),
            enabled: true,
            ..Default::default()
        },
    ]
}

impl AppConfig {
    /// Charge la configuration, crée le fichier par défaut s'il n'existe pas.
    pub fn load_or_create(path: &Path) -> IhResult<Self> {
        if path.exists() {
            let config = Self::load(path)?;
            config.validate()?;
            return Ok(config);
        }
        let config = Self::default();
        config.validate()?;
        config.save(path)?;
        Ok(config)
    }

    pub fn load(path: &Path) -> IhResult<Self> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            IhError::Config(format!("lecture de {} impossible : {e}", path.display()))
        })?;
        let config: Self = toml::from_str(&raw).map_err(|e| {
            IhError::Config(format!(
                "configuration illisible ({}) : {e}",
                path.display()
            ))
        })?;
        Ok(config)
    }

    /// Écriture atomique : fichier temporaire, synchronisation, renommage.
    pub fn save(&self, path: &Path) -> IhResult<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| IhError::Internal(format!("sérialisation TOML : {e}")))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, body)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn validate(&self) -> IhResult<()> {
        if !SUPPORTED_LOCALES.contains(&self.locale.as_str()) {
            return Err(IhError::Config(format!(
                "langue non prise en charge : {}",
                self.locale
            )));
        }
        if !matches!(self.theme.as_str(), "dark" | "light" | "system") {
            return Err(IhError::Config(format!("thème inconnu : {}", self.theme)));
        }
        self.resources.validate()?;

        let mut provider_ids = std::collections::BTreeSet::new();
        for provider in &self.providers {
            if provider.id.trim().is_empty() {
                return Err(IhError::Config("fournisseur sans identifiant".into()));
            }
            if !provider_ids.insert(provider.id.as_str()) {
                return Err(IhError::Config(format!(
                    "fournisseur dupliqué : {}",
                    provider.id
                )));
            }
            validate_url(&provider.base_url, provider.kind)?;
            if let Some(path) = &provider.endpoint_path {
                validate_endpoint_path(path)?;
            }
            validate_headers(&provider.headers)?;
            if let Some(temp) = provider.temperature {
                if !(0.0..=2.0).contains(&temp) {
                    return Err(IhError::Config(format!(
                        "temperature hors bornes (0–2) pour {}",
                        provider.id
                    )));
                }
            }
            if let Some(tokens) = provider.max_tokens {
                if tokens == 0 || tokens > 1_000_000 {
                    return Err(IhError::Config(format!(
                        "max_tokens hors bornes pour {}",
                        provider.id
                    )));
                }
            }
            if provider.timeout_secs == 0 || provider.timeout_secs > 3_600 {
                return Err(IhError::Config(format!(
                    "timeout fournisseur hors bornes : {}",
                    provider.id
                )));
            }
        }

        let mut agent_ids = std::collections::BTreeSet::new();
        for agent in &self.agents {
            if agent.id.trim().is_empty() {
                return Err(IhError::Config("agent sans identifiant".into()));
            }
            if !agent_ids.insert(agent.id.as_str()) {
                return Err(IhError::Config(format!("agent dupliqué : {}", agent.id)));
            }
            if agent.program.trim().is_empty() {
                return Err(IhError::Config(format!(
                    "agent sans programme : {}",
                    agent.id
                )));
            }
        }

        if let Some(endpoint) = &self.updates.endpoint {
            if !endpoint.starts_with("https://") {
                return Err(IhError::Config(
                    "l'endpoint de mise à jour doit être en HTTPS".into(),
                ));
            }
        }
        if self.updates.check_interval_minutes < 15 {
            return Err(IhError::Config(
                "intervalle de vérification des mises à jour trop court (min 15 min)".into(),
            ));
        }
        if let Some(key) = &self.updates.public_key_hex {
            if key.len() != 64 || hex::decode(key).is_err() {
                return Err(IhError::Config(
                    "clé publique Ed25519 invalide (64 caractères hexadécimaux attendus)".into(),
                ));
            }
        }
        if self.ui.editor_font_size < 9 || self.ui.editor_font_size > 32 {
            return Err(IhError::Config(
                "taille de police de l'éditeur hors bornes (9-32)".into(),
            ));
        }
        if !matches!(
            self.logging.level.as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            return Err(IhError::Config(format!(
                "niveau de journalisation inconnu : {}",
                self.logging.level
            )));
        }

        let workspace = &self.workspace;
        if workspace.max_context_files == 0 || workspace.max_context_files > 50 {
            return Err(IhError::Config(
                "workspace.max_context_files doit être entre 1 et 50".into(),
            ));
        }
        if workspace.max_context_bytes_per_file < 1024
            || workspace.max_context_bytes_per_file > 1024 * 1024
        {
            return Err(IhError::Config(
                "workspace.max_context_bytes_per_file doit être entre 1 Kio et 1 Mio".into(),
            ));
        }
        if workspace.max_context_bytes < workspace.max_context_bytes_per_file
            || workspace.max_context_bytes > 8 * 1024 * 1024
        {
            return Err(IhError::Config(
                "workspace.max_context_bytes doit être ≥ par fichier et ≤ 8 Mio".into(),
            ));
        }
        if workspace.max_editor_bytes < 4096 || workspace.max_editor_bytes > 8 * 1024 * 1024 {
            return Err(IhError::Config(
                "workspace.max_editor_bytes doit être entre 4 Kio et 8 Mio".into(),
            ));
        }
        for pattern in &workspace.exclusions {
            if pattern.trim().is_empty() || pattern.len() > 64 {
                return Err(IhError::Config(
                    "motif d'exclusion invalide (1 à 64 caractères)".into(),
                ));
            }
            if pattern.contains("..") || pattern.starts_with('/') {
                return Err(IhError::Config(format!(
                    "motif d'exclusion non autorisé : {pattern}"
                )));
            }
        }
        Ok(())
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn provider_mut(&mut self, id: &str) -> Option<&mut ProviderConfig> {
        self.providers.iter_mut().find(|p| p.id == id)
    }

    pub fn agent(&self, id: &str) -> Option<&AgentConfig> {
        self.agents.iter().find(|a| a.id == id)
    }

    /// Enregistre un projet récent (max 12, sans doublon).
    pub fn remember_project(&mut self, path: &str) {
        self.recent_projects.retain(|p| p != path);
        self.recent_projects.insert(0, path.to_string());
        self.recent_projects.truncate(12);
        self.last_project = Some(path.to_string());
    }
}

/// Vrai si l'hôte est local (localhost, boucle locale, réseau privé, `.local`).
pub fn is_local_host(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".local") || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}

fn validate_endpoint_path(path: &str) -> IhResult<()> {
    if !path.starts_with('/') {
        return Err(IhError::Config(
            "endpoint_path doit commencer par '/'".into(),
        ));
    }
    if path.len() > 256 || path.contains("..") || path.chars().any(|c| c.is_whitespace()) {
        return Err(IhError::Config(format!("endpoint_path invalide : {path}")));
    }
    Ok(())
}

fn validate_headers(headers: &std::collections::BTreeMap<String, String>) -> IhResult<()> {
    const RESERVED: [&str; 5] = [
        "authorization",
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
    ];
    for (name, value) in headers {
        if name.is_empty() || name.len() > 64 {
            return Err(IhError::Config(format!("nom d'en-tête invalide : {name}")));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err(IhError::Config(format!("nom d'en-tête invalide : {name}")));
        }
        if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(IhError::Config(format!(
                "en-tête réservé (utilisez la clé d'API du trousseau) : {name}"
            )));
        }
        if value.len() > 1024 || value.contains(['\r', '\n']) {
            return Err(IhError::Config(format!(
                "valeur d'en-tête invalide pour {name}"
            )));
        }
    }
    Ok(())
}

fn validate_url(url: &str, kind: ProviderKind) -> IhResult<()> {
    if url.trim().is_empty() {
        return Err(IhError::Config("URL de fournisseur vide".into()));
    }
    let host = url
        .split_once("://")
        .and_then(|(_, rest)| rest.split('/').next())
        .map(|authority| {
            let without_user = authority.rsplit('@').next().unwrap_or(authority);
            without_user.split(':').next().unwrap_or(without_user)
        })
        .unwrap_or("");

    // HTTPS exigé, sauf pour un hôte local explicitement configuré
    // (serveurs locaux : LM Studio, vLLM, Ollama, gateway privée…).
    let local_http =
        url.starts_with("http://") && (matches!(kind, ProviderKind::Ollama) || is_local_host(host));
    if !(url.starts_with("https://") || local_http) {
        return Err(IhError::Config(format!(
            "URL de fournisseur non autorisée (HTTPS requis hors hôte local) : {url}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ih-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn configuration_par_defaut_valide() {
        let config = AppConfig::default();
        config.validate().unwrap();
        assert_eq!(config.locale, "fr");
        assert_eq!(config.providers.len(), 5);
        assert_eq!(config.agents.len(), 5);
        assert!(config.providers.iter().all(|p| !p.has_secret));
    }

    #[test]
    fn aller_retour_disque() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        let mut config = AppConfig {
            locale: "en".into(),
            resources: ResourcePolicy {
                max_concurrent_agents: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        config.remember_project("/home/infina/projets/demo");
        config.save(&path).unwrap();

        let loaded = AppConfig::load(&path).unwrap();
        assert_eq!(loaded.locale, "en");
        assert_eq!(loaded.resources.max_concurrent_agents, 3);
        assert_eq!(
            loaded.last_project.as_deref(),
            Some("/home/infina/projets/demo")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn aucun_secret_dans_le_fichier() {
        let dir = temp_dir();
        let path = dir.join("config.toml");
        AppConfig::default().save(&path).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains("api_key"));
        assert!(!body.contains("sk-"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn langues_et_cles_invalides_refusees() {
        let config = AppConfig {
            locale: "de".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = AppConfig {
            updates: UpdateConfig {
                public_key_hex: Some("zz".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = AppConfig {
            updates: UpdateConfig {
                endpoint: Some("http://insecure.example/manifest.json".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn url_locale_autorisee_pour_ollama_mais_pas_pour_cloud() {
        let mut config = AppConfig::default();
        config.provider_mut("ollama").unwrap().base_url = "http://127.0.0.1:9999".into();
        config.validate().unwrap();

        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().base_url = "http://api.openai.com/v1".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn configuration_workspace_par_defaut_valide() {
        let config = AppConfig::default();
        config.validate().unwrap();
        assert_eq!(config.workspace.max_context_files, 12);
        assert!(config.workspace.restore_tabs);
        assert!(config.workspace.exclusions.is_empty());
    }

    #[test]
    fn configuration_workspace_hors_bornes_refusee() {
        let base = AppConfig::default();

        let mut config = base.clone();
        config.workspace.max_context_files = 0;
        assert!(config.validate().is_err());

        let mut config = base.clone();
        config.workspace.max_context_files = 500;
        assert!(config.validate().is_err());

        let mut config = base.clone();
        config.workspace.max_context_bytes = 1024;
        config.workspace.max_context_bytes_per_file = 65536;
        assert!(config.validate().is_err(), "total < par fichier");

        let mut config = base.clone();
        config.workspace.exclusions = vec!["../secret".into()];
        assert!(config.validate().is_err(), "motif avec .. refusé");

        let mut config = base;
        config.workspace.exclusions = vec!["node_modules".into(), "*.pem".into()];
        config.validate().unwrap();
    }

    #[test]
    fn hotes_locaux_reconnus() {
        for host in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "192.168.1.10",
            "10.0.0.5",
            "172.16.3.4",
            "192.168.0.22",
            "box.local",
        ] {
            assert!(is_local_host(host), "{host} doit être considéré local");
        }
        for host in [
            "api.openai.com",
            "openrouter.ai",
            "8.8.8.8",
            "example.invalid",
        ] {
            assert!(!is_local_host(host), "{host} ne doit pas être local");
        }
    }

    #[test]
    fn http_autorise_pour_un_hote_local_uniquement() {
        let mut config = AppConfig::default();
        config.provider_mut("ollama").unwrap().base_url = "http://192.168.1.50:11434".into();
        config.validate().unwrap();

        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().base_url = "http://192.168.1.50:8000/v1".into();
        config.validate().unwrap();

        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().base_url = "http://api.openai.com/v1".into();
        assert!(config.validate().is_err(), "HTTP distant refusé");
    }

    #[test]
    fn endpoint_personnalise_valide() {
        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().endpoint_path = Some("/v1/chat/completions".into());
        config.validate().unwrap();

        for path in ["v1/chat", "/v1/../secret", "/v1/chat completions"] {
            let mut config = AppConfig::default();
            config.provider_mut("openai").unwrap().endpoint_path = Some(path.into());
            assert!(config.validate().is_err(), "chemin refusé : {path}");
        }
    }

    #[test]
    fn entetes_personnalises_valides_et_injections_refusees() {
        let mut config = AppConfig::default();
        let provider = config.provider_mut("openai").unwrap();
        provider.headers.insert("X-Tenant".into(), "acme".into());
        provider
            .headers
            .insert("X-Trace-Id".into(), "abc-123".into());
        config.validate().unwrap();

        // En-tête réservé : la clé doit passer par le trousseau.
        let mut config = AppConfig::default();
        config
            .provider_mut("openai")
            .unwrap()
            .headers
            .insert("Authorization".into(), "Bearer x".into());
        assert!(config.validate().is_err());

        // Injection CRLF.
        let mut config = AppConfig::default();
        config
            .provider_mut("openai")
            .unwrap()
            .headers
            .insert("X-Test".into(), "a\r\nHost: evil".into());
        assert!(config.validate().is_err());

        // Nom invalide.
        let mut config = AppConfig::default();
        config
            .provider_mut("openai")
            .unwrap()
            .headers
            .insert("X Test".into(), "v".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn parametres_de_generation_bornees() {
        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().temperature = Some(1.5);
        config.provider_mut("openai").unwrap().max_tokens = Some(2048);
        config.validate().unwrap();

        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().temperature = Some(3.0);
        assert!(config.validate().is_err());

        let mut config = AppConfig::default();
        config.provider_mut("openai").unwrap().max_tokens = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn projets_recents_sans_doublon() {
        let mut config = AppConfig::default();
        config.remember_project("/a");
        config.remember_project("/b");
        config.remember_project("/a");
        assert_eq!(config.recent_projects, vec!["/a", "/b"]);
    }
}
