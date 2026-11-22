//! Manifeste de mise à jour : parsing, validation et comparaison de versions.
//!
//! Le manifeste est un document JSON signé en détaché (voir
//! [`crate::verify_signature`]). Ce module ne fait aucune E/S : il transforme
//! des octets déjà authentifiés en structures utilisables, en refusant tout ce
//! qui sort du contrat.

use chrono::{DateTime, Utc};
use ih_core::{IhError, IhResult};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::MAX_DOWNLOAD_BYTES;

/// Seule version de schéma comprise par cette implémentation.
pub const SUPPORTED_SCHEMA: u32 = 1;

/// Type d'artefact installable sur Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    AppImage,
    Deb,
    Pacman,
}

/// Un artefact publié pour une plateforme.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// URL HTTPS de l'artefact (HTTP est refusé).
    pub url: String,
    /// Empreinte SHA-256 hexadécimale (64 caractères).
    pub sha256: String,
    /// Taille annoncée, vérifiée après téléchargement lorsqu'elle est fournie.
    #[serde(default)]
    pub size: Option<u64>,
    pub kind: ArtifactKind,
}

/// Manifeste de publication, tel que signé par la CI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub schema: u32,
    pub version: String,
    pub channel: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub released_at: Option<DateTime<Utc>>,
    pub platforms: BTreeMap<String, Artifact>,
}

/// Clé de plateforme canonique associée à un type d'artefact.
pub fn target_key(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::AppImage => "linux-x86_64-appimage",
        ArtifactKind::Deb => "linux-x86_64-deb",
        ArtifactKind::Pacman => "linux-x86_64-pacman",
    }
}

/// Analyse et valide un manifeste.
///
/// Refuse : schéma inconnu, version non semver, canal vide, aucune plateforme,
/// URL non HTTPS, empreinte malformée, taille aberrante et type d'artefact
/// inconnu (ce dernier est rejeté par serde).
pub fn parse_manifest(bytes: &[u8]) -> IhResult<UpdateManifest> {
    let mut manifest: UpdateManifest = serde_json::from_slice(bytes)
        .map_err(|err| update_error(format!("manifeste JSON invalide : {err}")))?;

    if manifest.schema != SUPPORTED_SCHEMA {
        return Err(update_error(format!(
            "schéma de manifeste non pris en charge : {} (attendu : {SUPPORTED_SCHEMA})",
            manifest.schema
        )));
    }

    if let Err(err) = Version::parse(&manifest.version) {
        return Err(update_error(format!(
            "version de manifeste non semver ({:?}) : {err}",
            manifest.version
        )));
    }

    if manifest.channel.trim().is_empty() {
        return Err(update_error("canal de manifeste vide"));
    }

    if manifest.platforms.is_empty() {
        return Err(update_error("le manifeste ne publie aucune plateforme"));
    }

    for (key, artifact) in manifest.platforms.iter_mut() {
        if key.trim().is_empty() {
            return Err(update_error("clé de plateforme vide dans le manifeste"));
        }
        validate_artifact_url(&artifact.url)?;
        artifact.sha256 = crate::crypto::normalize_sha256(&artifact.sha256)?;
        if let Some(size) = artifact.size {
            if size == 0 {
                return Err(update_error(format!(
                    "taille nulle pour la plateforme {key}"
                )));
            }
            if size > MAX_DOWNLOAD_BYTES {
                return Err(update_error(format!(
                    "taille annoncée trop grande pour la plateforme {key} ({size} octets, maximum {MAX_DOWNLOAD_BYTES})"
                )));
            }
        }
    }

    Ok(manifest)
}

/// Indique si `candidate` est strictement plus récente que `current_version`.
///
/// Les deux versions doivent être du semver strict ; une version invalide est
/// une erreur. Une version plus ancienne ou égale renvoie `false` : ce crate
/// refuse tout downgrade.
pub fn is_newer(current_version: &str, candidate: &str) -> IhResult<bool> {
    let current = Version::parse(current_version).map_err(|err| {
        update_error(format!(
            "version courante non semver ({current_version:?}) : {err}"
        ))
    })?;
    let candidate = Version::parse(candidate).map_err(|err| {
        update_error(format!(
            "version candidate non semver ({candidate:?}) : {err}"
        ))
    })?;
    // `cmp_precedence` applique la précédence SemVer : les métadonnées de
    // build (`+sha`) ne rendent pas une version plus récente.
    Ok(candidate.cmp_precedence(&current) == std::cmp::Ordering::Greater)
}

fn update_error(message: impl Into<String>) -> IhError {
    IhError::Update(message.into())
}

fn validate_artifact_url(raw: &str) -> IhResult<()> {
    let url = reqwest::Url::parse(raw)
        .map_err(|err| update_error(format!("URL d'artefact invalide : {err}")))?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(update_error(
            "URL d'artefact avec identifiants intégrés refusée",
        ));
    }

    match url.scheme() {
        "https" => Ok(()),
        "http" => Err(update_error(format!(
            "URL d'artefact non sécurisée (HTTPS requis) : {raw}"
        ))),
        other => Err(update_error(format!(
            "schéma d'URL d'artefact refusé : {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> serde_json::Value {
        serde_json::json!({
            "schema": 1,
            "version": "0.2.0",
            "channel": "stable",
            "notes": "Corrections",
            "released_at": "2026-11-22T10:00:00Z",
            "platforms": {
                "linux-x86_64-appimage": {
                    "url": "https://releases.example.com/IH_0.2.0.AppImage",
                    "sha256": "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
                    "size": 12345678,
                    "kind": "appimage"
                },
                "linux-x86_64-deb": {
                    "url": "https://releases.example.com/ih_0.2.0_amd64.deb",
                    "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                    "size": 123,
                    "kind": "deb"
                },
                "linux-x86_64-pacman": {
                    "url": "https://releases.example.com/IH-0.2.0-1-x86_64.pkg.tar.zst",
                    "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                    "size": 123,
                    "kind": "pacman"
                }
            }
        })
    }

    fn parse_value(value: &serde_json::Value) -> IhResult<UpdateManifest> {
        parse_manifest(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn manifeste_valide_accepte_et_normalise() {
        let manifest = parse_value(&valid_json()).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.channel, "stable");
        assert!(manifest.released_at.is_some());
        assert_eq!(manifest.platforms.len(), 3);

        let appimage = &manifest.platforms["linux-x86_64-appimage"];
        assert_eq!(appimage.kind, ArtifactKind::AppImage);
        assert_eq!(appimage.sha256, appimage.sha256.to_ascii_lowercase());
        assert_eq!(appimage.size, Some(12_345_678));
    }

    #[test]
    fn schema_inconnu_refuse() {
        let mut value = valid_json();
        value["schema"] = serde_json::json!(2);
        assert!(parse_value(&value).is_err());
    }

    #[test]
    fn version_non_semver_refusee() {
        for version in ["0.2", "v0.2.0", "pas-une-version", ""] {
            let mut value = valid_json();
            value["version"] = serde_json::json!(version);
            assert!(parse_value(&value).is_err(), "version {version:?}");
        }
    }

    #[test]
    fn url_http_refusee_meme_en_boucle_locale() {
        for url in [
            "http://releases.example.com/IH.AppImage",
            "http://127.0.0.1:8080/IH.AppImage",
            "http://localhost:8080/IH.AppImage",
            "http://[::1]:8080/IH.AppImage",
        ] {
            let mut value = valid_json();
            value["platforms"]["linux-x86_64-appimage"]["url"] = serde_json::json!(url);
            assert!(parse_value(&value).is_err(), "url {url:?}");
        }
    }

    #[test]
    fn url_non_https_refusee() {
        for url in [
            "ftp://releases.example.com/IH.AppImage",
            "file:///tmp/IH.AppImage",
            "https://",
        ] {
            let mut value = valid_json();
            value["platforms"]["linux-x86_64-appimage"]["url"] = serde_json::json!(url);
            assert!(parse_value(&value).is_err(), "url {url:?}");
        }
    }

    #[test]
    fn sha256_malforme_refuse() {
        for digest in [
            String::new(),
            "abc".to_string(),
            "z".repeat(64),
            "a".repeat(63),
            "a".repeat(65),
        ] {
            let mut value = valid_json();
            value["platforms"]["linux-x86_64-appimage"]["sha256"] = serde_json::json!(digest);
            assert!(parse_value(&value).is_err(), "digest {digest:?}");
        }
    }

    #[test]
    fn kind_inconnu_refuse() {
        let mut value = valid_json();
        value["platforms"]["linux-x86_64-appimage"]["kind"] = serde_json::json!("snap");
        assert!(parse_value(&value).is_err());
    }

    #[test]
    fn taille_aberrante_refusee() {
        let mut value = valid_json();
        value["platforms"]["linux-x86_64-appimage"]["size"] = serde_json::json!(0);
        assert!(parse_value(&value).is_err());

        let mut value = valid_json();
        value["platforms"]["linux-x86_64-appimage"]["size"] =
            serde_json::json!(MAX_DOWNLOAD_BYTES + 1);
        assert!(parse_value(&value).is_err());
    }

    #[test]
    fn cles_de_plateforme_canoniques() {
        assert_eq!(target_key(ArtifactKind::AppImage), "linux-x86_64-appimage");
        assert_eq!(target_key(ArtifactKind::Deb), "linux-x86_64-deb");
        assert_eq!(target_key(ArtifactKind::Pacman), "linux-x86_64-pacman");
    }

    #[test]
    fn comparaison_de_versions_stricte() {
        assert!(is_newer("0.2.0", "0.2.1").unwrap());
        assert!(is_newer("0.2.0", "0.3.0").unwrap());
        assert!(!is_newer("0.3.0", "0.2.9").unwrap());
        assert!(!is_newer("0.2.0", "0.2.0").unwrap());
        assert!(!is_newer("1.0.0", "1.0.0+build.7").unwrap());
        assert!(is_newer("1.0.0-alpha", "1.0.0").unwrap());
        assert!(!is_newer("1.0.0", "1.0.0-alpha").unwrap());
    }

    #[test]
    fn comparaison_de_versions_invalides_refusee() {
        assert!(is_newer("0.2", "0.3.0").is_err());
        assert!(is_newer("0.2.0", "v0.3.0").is_err());
        assert!(is_newer("pas-une-version", "0.3.0").is_err());
    }
}
