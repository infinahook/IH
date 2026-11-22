//! Journalisation fichier + console, avec rotation quotidienne.
//!
//! Les messages passent par `ih_core::redaction` avant d'atteindre le disque
//! pour les commandes et les erreurs de fournisseur : voir `redact_guard`.

use ih_core::{AppPaths, IhResult};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Initialise la journalisation. Le guard doit vivre aussi longtemps que
/// l'application, sinon les dernières lignes ne sont pas écrites.
pub fn init(paths: &AppPaths, level: &str) -> IhResult<Option<WorkerGuard>> {
    std::fs::create_dir_all(paths.log_dir())?;
    let file_appender = tracing_appender::rolling::daily(paths.log_dir(), "ih.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_new(format!("ih={level},ih_core={level},{level}"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(non_blocking);
    let console_layer = tracing_subscriber::fmt::layer().with_target(false);

    if tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .try_init()
        .is_err()
    {
        // Un abonné est déjà installé (tests, hot reload) : ce n'est pas fatal.
        tracing::debug!("journalisation déjà initialisée");
    }
    Ok(Some(guard))
}
