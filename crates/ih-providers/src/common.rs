//! Plomberie HTTP commune aux fournisseurs : construction du client, URLs,
//! normalisation des erreurs et télémétrie de santé.

use crate::{HealthReport, Usage};
use ih_core::config::{ModelEntry, ProviderConfig, ProviderKind};
use ih_core::{IhError, IhResult};
use reqwest::{Response, StatusCode};
use std::time::{Duration, Instant};

/// Valeur de repli quand l'API exige `max_tokens` mais que l'appelant l'omet.
pub(crate) const DEFAULT_MAX_TOKENS: u32 = 1024;

/// Valide les invariants communs avant toute construction de client.
pub(crate) fn validate_config(config: &ProviderConfig, expected: ProviderKind) -> IhResult<()> {
    if config.kind != expected {
        let id = if config.id.trim().is_empty() {
            "<sans id>"
        } else {
            config.id.as_str()
        };
        return Err(IhError::Config(format!(
            "fournisseur {id} : type {:?} attendu, reçu {:?}",
            expected, config.kind
        )));
    }
    if config.id.trim().is_empty() {
        return Err(IhError::Config("fournisseur sans identifiant".into()));
    }
    if config.base_url.trim().is_empty() {
        return Err(IhError::Config(format!(
            "URL de fournisseur vide : {}",
            config.id
        )));
    }
    if config.timeout_secs == 0 {
        return Err(IhError::Config(format!(
            "timeout fournisseur nul : {}",
            config.id
        )));
    }
    Ok(())
}

/// Refuse une configuration cloud sans secret, avant toute connexion.
pub(crate) fn require_secret(config: &ProviderConfig, secret: Option<String>) -> IhResult<String> {
    match secret {
        Some(secret) if !secret.trim().is_empty() => Ok(secret.trim().to_string()),
        _ => Err(IhError::Provider(format!(
            "clé d'API absente pour {}",
            config.id
        ))),
    }
}

/// Vérifie que `base_url` est une URL HTTP(S) exploitable.
pub(crate) fn validate_base_url(config: &ProviderConfig) -> IhResult<()> {
    let url = reqwest::Url::parse(config.base_url.trim()).map_err(|error| {
        IhError::Config(format!(
            "URL de fournisseur invalide pour {} : {error}",
            config.id
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(IhError::Config(format!(
            "URL de fournisseur non HTTP(S) pour {} : {}",
            config.id, config.base_url
        )));
    }
    if url.host_str().is_none() {
        return Err(IhError::Config(format!(
            "URL de fournisseur sans hôte pour {} : {}",
            config.id, config.base_url
        )));
    }
    Ok(())
}

/// Construit le client avec le timeout de la configuration.
pub(crate) fn build_client(timeout_secs: u64) -> IhResult<reqwest::Client> {
    let timeout = Duration::from_secs(timeout_secs.max(1));
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("IH/", env!("CARGO_PKG_VERSION")))
        // Aucune redirection suivie : une redirection pourrait envoyer la clé
        // d'API vers un autre hôte. Le code 3xx est remonté comme erreur.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| IhError::Provider(format!("client HTTP impossible à créer : {error}")))
}

/// Joint un chemin à une base sans doubler les barres obliques.
pub(crate) fn endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Joint un chemin versionné (`v1`, `v1beta`) sans dupliquer le préfixe si la
/// `base_url` le contient déjà (par exemple `https://api.anthropic.com/v1`).
pub(crate) fn versioned_endpoint(base: &str, version: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let suffix = format!("/{version}");
    if base.ends_with(&suffix) {
        endpoint(base, path)
    } else {
        endpoint(&format!("{base}{suffix}"), path)
    }
}

/// Ajoute les en-têtes HTTP configurés par l'utilisateur (jamais de secret :
/// la validation de configuration interdit `authorization`/`host`).
pub(crate) fn apply_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &[(String, String)],
) -> reqwest::RequestBuilder {
    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
}

/// URL d'un endpoint personnalisé : origine de `base_url` + chemin absolu.
pub(crate) fn custom_endpoint_url(base: &str, path: &str) -> String {
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    let authority = rest.split('/').next().unwrap_or(rest);
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{scheme}://{authority}{path}")
}

/// En-têtes configurés, sous forme de paires.
pub(crate) fn header_pairs(config: &ProviderConfig) -> Vec<(String, String)> {
    config
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// Envoie la requête en normalisant les erreurs réseau.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    provider_id: &str,
    timeout_secs: u64,
) -> IhResult<Response> {
    tracing::debug!(
        provider = provider_id,
        timestamp = %chrono::Utc::now().to_rfc3339(),
        "requête HTTP fournisseur"
    );
    request
        .send()
        .await
        .map_err(|error| network_error(provider_id, timeout_secs, &error))
}

/// Convertit une erreur `reqwest` sans jamais exposer d'en-tête d'autorisation.
pub(crate) fn network_error(
    provider_id: &str,
    timeout_secs: u64,
    error: &reqwest::Error,
) -> IhError {
    if error.is_timeout() {
        IhError::Network(format!(
            "délai dépassé ({timeout_secs} s) pour {provider_id}"
        ))
    } else if error.is_connect() {
        IhError::Network(format!(
            "connexion au fournisseur {provider_id} impossible : {error}"
        ))
    } else {
        IhError::Network(format!("erreur réseau pour {provider_id} : {error}"))
    }
}

/// Transforme une réponse non 2xx en [`IhError::Provider`] explicite.
pub(crate) async fn ensure_success(response: Response, provider_id: &str) -> IhResult<Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(http_error(provider_id, status, &body))
}

/// Message utilisateur stable selon le code HTTP, enrichi du message fournisseur.
pub(crate) fn http_error(provider_id: &str, status: StatusCode, body: &str) -> IhError {
    let detail = extract_provider_error(body).unwrap_or_else(|| truncate(body.trim(), 300));
    let summary = match status.as_u16() {
        401 => "clé d'API invalide",
        403 => "accès refusé par le fournisseur",
        404 => "endpoint introuvable",
        429 => "quota/limite atteint",
        500..=599 => "erreur serveur du fournisseur",
        _ => "erreur HTTP du fournisseur",
    };
    let code = status.as_u16();
    if detail.is_empty() {
        IhError::Provider(format!("{summary} ({code}) pour {provider_id}"))
    } else {
        IhError::Provider(format!("{summary} ({code}) pour {provider_id} : {detail}"))
    }
}

/// Extrait le message d'erreur JSON du fournisseur s'il est présent.
fn extract_provider_error(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let error = value.get("error").unwrap_or(&value);
    let message = match error {
        serde_json::Value::String(message) => Some(message.as_str()),
        serde_json::Value::Object(map) => map
            .get("message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| map.get("detail").and_then(serde_json::Value::as_str))
            .or_else(|| map.get("msg").and_then(serde_json::Value::as_str)),
        _ => None,
    };
    message.map(str::to_string)
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

/// Détecte un objet `error` glissé dans un flux pourtant en HTTP 200.
pub(crate) fn check_stream_payload(provider_id: &str, value: &serde_json::Value) -> IhResult<()> {
    let Some(error) = value.get("error") else {
        return Ok(());
    };
    let detail = match error {
        serde_json::Value::String(message) => message.clone(),
        serde_json::Value::Object(map) => map
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("erreur du fournisseur")
            .to_string(),
        other => other.to_string(),
    };
    Err(IhError::Provider(format!(
        "erreur du fournisseur {provider_id} : {detail}"
    )))
}

/// Décode une réponse JSON complète avec un message d'erreur contextualisé.
pub(crate) fn parse_provider_json<T: serde::de::DeserializeOwned>(
    provider_id: &str,
    body: &str,
) -> IhResult<T> {
    serde_json::from_str(body)
        .map_err(|error| IhError::Provider(format!("réponse {provider_id} invalide : {error}")))
}

/// Décode un fragment de flux JSON.
pub(crate) fn parse_stream_json<T: serde::de::DeserializeOwned>(
    provider_id: &str,
    data: &str,
) -> IhResult<T> {
    serde_json::from_str(data).map_err(|error| {
        IhError::Provider(format!(
            "fragment de flux invalide pour {provider_id} : {error}"
        ))
    })
}

/// Santé calculée à partir d'un `list_models` réussi ou en échec.
pub(crate) fn health_from_models(
    started: Instant,
    result: IhResult<Vec<ModelEntry>>,
) -> HealthReport {
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(models) => HealthReport {
            ok: true,
            latency_ms,
            detail: format!("{} modèle(s) disponible(s)", models.len()),
        },
        Err(error) => HealthReport {
            ok: false,
            latency_ms,
            detail: error.to_string(),
        },
    }
}

/// Renseigne l'usage sans écraser une valeur déjà connue par `None`.
pub(crate) fn merge_usage(usage: &mut Usage, input: Option<u32>, output: Option<u32>) {
    if let Some(value) = input {
        usage.input_tokens = Some(value);
    }
    if let Some(value) = output {
        usage.output_tokens = Some(value);
    }
}
