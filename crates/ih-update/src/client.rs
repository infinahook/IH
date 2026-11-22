//! Client HTTP de mise à jour.
//!
//! Toutes les écritures se font dans un fichier `.part` du dossier choisi par
//! l'appelant ; le fichier final n'apparaît qu'après vérification de la taille
//! et de l'empreinte SHA-256. Un échec supprime le fichier partiel.

use crate::crypto::{constant_time_eq, normalize_sha256, verify_signature};
use crate::manifest::{parse_manifest, Artifact, UpdateManifest};
use futures_util::StreamExt;
use ih_core::{IhError, IhResult};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Taille maximale d'un artefact téléchargé (500 Mio).
pub const MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;
/// Taille maximale du manifeste (1 Mio).
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// Taille maximale du fichier de signature détachée.
pub const MAX_SIGNATURE_BYTES: u64 = 8 * 1024;

/// Client de récupération et de vérification des mises à jour.
#[derive(Clone, Debug)]
pub struct UpdateClient {
    http: reqwest::Client,
}

impl UpdateClient {
    /// Construit un client HTTP avec des délais bornés et des redirections
    /// limitées.
    pub fn new() -> IhResult<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("IH/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(1_800))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|err| {
                IhError::Network(format!("création du client HTTP impossible : {err}"))
            })?;
        Ok(Self { http })
    }

    /// Récupère `manifest.json` et `manifest.json.sig`, vérifie la signature
    /// Ed25519 sur les octets bruts, puis valide le manifeste.
    ///
    /// Retourne le manifeste et les octets exacts qui ont été signés (utiles
    /// pour l'audit ou une nouvelle vérification).
    pub async fn fetch_verified(
        &self,
        manifest_url: &str,
        signature_url: &str,
        public_key_hex: &str,
    ) -> IhResult<(UpdateManifest, Vec<u8>)> {
        let manifest_bytes = self
            .get_bytes(manifest_url, MAX_MANIFEST_BYTES, "manifeste")
            .await?;
        let signature_bytes = self
            .get_bytes(signature_url, MAX_SIGNATURE_BYTES, "signature détachée")
            .await?;
        let signature_hex = std::str::from_utf8(&signature_bytes)
            .map_err(|_| update_error("signature détachée : contenu non UTF-8"))?;

        verify_signature(&manifest_bytes, signature_hex.trim(), public_key_hex)?;
        let manifest = parse_manifest(&manifest_bytes)?;

        tracing::info!(
            version = %manifest.version,
            channel = %manifest.channel,
            "manifeste de mise à jour vérifié"
        );
        Ok((manifest, manifest_bytes))
    }

    /// Télécharge un artefact dans `dest` en calculant son SHA-256 au fil de
    /// l'eau.
    ///
    /// Le flux est plafonné à [`MAX_DOWNLOAD_BYTES`]. Les données sont écrites
    /// dans `dest.part`, comparées à l'empreinte (et à la taille annoncée si
    /// présente), puis seulement renommées vers `dest`. Toute erreur supprime
    /// le fichier partiel.
    pub async fn download(&self, artifact: &Artifact, dest: &Path) -> IhResult<()> {
        let expected_digest = normalize_sha256(&artifact.sha256)?;
        if artifact.size.is_some_and(|size| size > MAX_DOWNLOAD_BYTES) {
            return Err(IhError::Limit(format!(
                "taille annoncée supérieure à la limite de {MAX_DOWNLOAD_BYTES} octets"
            )));
        }
        if dest.file_name().is_none() {
            return Err(update_error(
                "destination de téléchargement sans nom de fichier",
            ));
        }
        if let Some(parent) = dest.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }

        let part = part_path(dest);
        match tokio::fs::remove_file(&part).await {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        let result = self
            .download_to_part(artifact, &expected_digest, &part, dest)
            .await;
        if let Err(err) = result {
            let _ = tokio::fs::remove_file(&part).await;
            tracing::warn!(
                url = %redact_url(&artifact.url),
                "téléchargement échoué, fichier partiel supprimé"
            );
            return Err(err);
        }
        Ok(())
    }

    async fn download_to_part(
        &self,
        artifact: &Artifact,
        expected_digest: &str,
        part: &Path,
        dest: &Path,
    ) -> IhResult<()> {
        let response = self
            .http
            .get(&artifact.url)
            .send()
            .await
            .map_err(|err| network_error(describe_reqwest(&err), &artifact.url))?;
        let status = response.status();
        if !status.is_success() {
            return Err(network_error(
                format!("réponse HTTP {status}"),
                &artifact.url,
            ));
        }
        if let Some(length) = response.content_length() {
            if length > MAX_DOWNLOAD_BYTES {
                return Err(IhError::Limit(format!(
                    "artefact trop volumineux : {length} octets annoncés, maximum {MAX_DOWNLOAD_BYTES}"
                )));
            }
            if let Some(expected_size) = artifact.size {
                if length != expected_size {
                    return Err(update_error(format!(
                        "taille HTTP incohérente : {length} octets reçus, {expected_size} annoncés par le manifeste"
                    )));
                }
            }
        }

        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(part)
            .await?;
        let mut hasher = Sha256::new();
        let mut total: u64 = 0;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|err| network_error(describe_reqwest(&err), &artifact.url))?;
            total = total
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| IhError::Limit("taille de téléchargement débordante".into()))?;
            if total > MAX_DOWNLOAD_BYTES {
                return Err(IhError::Limit(format!(
                    "artefact trop volumineux : limite de {MAX_DOWNLOAD_BYTES} octets dépassée"
                )));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        if let Some(expected_size) = artifact.size {
            if total != expected_size {
                return Err(update_error(format!(
                    "taille téléchargée inattendue : {total} octets, {expected_size} annoncés"
                )));
            }
        }

        let actual = hex::encode(hasher.finalize());
        if !constant_time_eq(actual.as_bytes(), expected_digest.as_bytes()) {
            return Err(update_error(format!(
                "empreinte SHA-256 de l'artefact non conforme ({})",
                redact_url(&artifact.url)
            )));
        }

        // Même dossier : le renommage est atomique.
        tokio::fs::rename(part, dest).await?;
        tracing::info!(
            url = %redact_url(&artifact.url),
            bytes = total,
            "artefact téléchargé et vérifié"
        );
        Ok(())
    }

    async fn get_bytes(&self, url: &str, max: u64, label: &str) -> IhResult<Vec<u8>> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| network_error(describe_reqwest(&err), url))?;
        let status = response.status();
        if !status.is_success() {
            return Err(network_error(format!("réponse HTTP {status}"), url));
        }
        if let Some(length) = response.content_length() {
            if length > max {
                return Err(IhError::Limit(format!(
                    "{label} trop volumineux : {length} octets, maximum {max}"
                )));
            }
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|err| network_error(describe_reqwest(&err), url))?;
            let next_len = body.len() as u64 + chunk.len() as u64;
            if next_len > max {
                return Err(IhError::Limit(format!(
                    "{label} trop volumineux : limite de {max} octets dépassée"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

/// Résumé sûr d'une erreur reqwest : le message d'origine peut contenir l'URL
/// complète (avec jeton éventuel), il n'est donc jamais journalisé tel quel.
fn describe_reqwest(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "délai réseau dépassé"
    } else if err.is_connect() {
        "connexion impossible"
    } else if err.is_decode() {
        "réponse illisible"
    } else if err.is_body() {
        "flux de réponse interrompu"
    } else if err.is_request() {
        "requête HTTP invalide"
    } else {
        "erreur réseau"
    }
}

/// Retire requête, fragment et identifiants d'une URL avant journalisation.
fn redact_url(raw: &str) -> String {
    match reqwest::Url::parse(raw) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "<URL invalide>".to_string(),
    }
}

fn network_error(detail: impl std::fmt::Display, url: &str) -> IhError {
    IhError::Network(format!("{detail} ({})", redact_url(url)))
}

fn update_error(message: impl Into<String>) -> IhError {
    IhError::Update(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ArtifactKind;
    use crate::test_support::{
        manifest_json, public_key_hex, sha256_hex, sign_hex, test_signing_key, TestServer,
    };
    use sha2::{Digest, Sha256};

    fn artifact(url: String, body: &[u8], digest: Option<String>) -> Artifact {
        Artifact {
            url,
            sha256: digest.unwrap_or_else(|| sha256_hex(body)),
            size: Some(body.len() as u64),
            kind: ArtifactKind::AppImage,
        }
    }

    fn part_of(dest: &Path) -> PathBuf {
        part_path(dest)
    }

    #[tokio::test]
    async fn telechargement_verifie_fichier_present() {
        let body = b"contenu-appimage-de-test".to_vec();
        let server = TestServer::new(vec![("/IH.AppImage", body.clone())]);
        let client = UpdateClient::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("staging").join("IH.AppImage");

        client
            .download(&artifact(server.url("/IH.AppImage"), &body, None), &dest)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!part_of(&dest).exists());
    }

    #[tokio::test]
    async fn empreinte_incorrecte_supprime_le_fichier_partiel() {
        let body = b"contenu-appimage-de-test".to_vec();
        let server = TestServer::new(vec![("/IH.AppImage", body.clone())]);
        let client = UpdateClient::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("IH.AppImage");

        let bad = artifact(server.url("/IH.AppImage"), &body, Some("0".repeat(64)));
        let err = client.download(&bad, &dest).await.unwrap_err();

        assert!(err.to_string().contains("SHA-256"), "{err}");
        assert!(!dest.exists());
        assert!(!part_of(&dest).exists());
    }

    #[tokio::test]
    async fn taille_incoherente_refusee_et_partiel_supprime() {
        let body = b"artefact".to_vec();
        let server = TestServer::new(vec![("/IH.AppImage", body.clone())]);
        let client = UpdateClient::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("IH.AppImage");
        let mut bad = artifact(server.url("/IH.AppImage"), &body, None);
        bad.size = Some(body.len() as u64 + 10);

        assert!(client.download(&bad, &dest).await.is_err());
        assert!(!dest.exists());
        assert!(!part_of(&dest).exists());
    }

    #[tokio::test]
    async fn erreur_http_refusee() {
        let server = TestServer::new(vec![]);
        let client = UpdateClient::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("IH.AppImage");
        let bad = artifact(server.url("/absent"), b"x", None);

        let err = client.download(&bad, &dest).await.unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");
        assert!(!dest.exists());
        assert!(!part_of(&dest).exists());
    }

    #[tokio::test]
    async fn digest_calcule_au_fil_de_l_eau() {
        // Vérifie que l'empreinte est bien calculée sur le flux complet, même
        // avec des données binaires non UTF-8.
        let body: Vec<u8> = (0u8..=255).cycle().take(10_000).collect();
        let server = TestServer::new(vec![("/blob", body.clone())]);
        let client = UpdateClient::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("blob");

        let expected = hex::encode(Sha256::digest(&body));
        client
            .download(&artifact(server.url("/blob"), &body, Some(expected)), &dest)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }

    #[test]
    fn url_journalisee_sans_identifiants_ni_requete() {
        let redacted = redact_url(
            "https://utilisateur:jeton-secret@example.com/IH.AppImage?token=secret#fragment",
        );
        assert!(!redacted.contains("utilisateur"), "{redacted}");
        assert!(!redacted.contains("jeton-secret"), "{redacted}");
        assert!(!redacted.contains("secret"), "{redacted}");
        assert!(!redacted.contains("fragment"), "{redacted}");
        assert_eq!(redacted, "https://example.com/IH.AppImage");
    }

    #[tokio::test]
    async fn fetch_verifie_le_manifeste_signe() {
        let key = test_signing_key();
        let body = manifest_json(
            "0.2.0",
            "https://releases.example.com/IH.AppImage",
            &"a".repeat(64),
        );
        let signature = sign_hex(&key, &body);
        let server = TestServer::new(vec![
            ("/manifest.json", body.clone()),
            ("/manifest.json.sig", signature.into_bytes()),
        ]);

        let client = UpdateClient::new().unwrap();
        let (manifest, raw) = client
            .fetch_verified(
                &server.url("/manifest.json"),
                &server.url("/manifest.json.sig"),
                &public_key_hex(&key),
            )
            .await
            .unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(raw, body);
    }

    #[tokio::test]
    async fn fetch_refuse_un_manifeste_altere_apres_signature() {
        let key = test_signing_key();
        let original = manifest_json(
            "0.2.0",
            "https://releases.example.com/IH.AppImage",
            &"a".repeat(64),
        );
        let signature = sign_hex(&key, &original);
        let altered = String::from_utf8(original)
            .unwrap()
            .replace("0.2.0", "0.2.1")
            .into_bytes();
        let server = TestServer::new(vec![
            ("/manifest.json", altered),
            ("/manifest.json.sig", signature.into_bytes()),
        ]);

        let client = UpdateClient::new().unwrap();
        let err = client
            .fetch_verified(
                &server.url("/manifest.json"),
                &server.url("/manifest.json.sig"),
                &public_key_hex(&key),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
    }
}
