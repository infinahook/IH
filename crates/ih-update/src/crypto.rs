//! Vérifications cryptographiques : signature Ed25519 détachée et SHA-256.
//!
//! Aucune de ces fonctions ne journalise de clé privée, de signature complète
//! ni de contenu binaire : seuls des états et des chemins sont tracés.

use ed25519_dalek::{Signature, VerifyingKey};
use ih_core::{IhError, IhResult};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

const PUBLIC_KEY_LEN: usize = 32;
const SIGNATURE_LEN: usize = 64;
const SHA256_HEX_LEN: usize = 64;
const READ_BUFFER_LEN: usize = 64 * 1024;

/// Vérifie une signature Ed25519 détachée sur les octets **exacts** d'un
/// manifeste.
///
/// `signature_hex` et `public_key_hex` sont de l'hexadécimal strict (la
/// signature fait 64 octets, la clé publique 32). La vérification utilise
/// `verify_strict` : une signature sur d'autres octets, une clé invalide, une
/// signature de mauvaise longueur ou un manifeste modifié après signature sont
/// refusés avec un message explicite.
pub fn verify_signature(
    manifest_bytes: &[u8],
    signature_hex: &str,
    public_key_hex: &str,
) -> IhResult<()> {
    let key_bytes = decode_fixed_hex(
        public_key_hex.trim(),
        PUBLIC_KEY_LEN,
        "clé publique Ed25519",
    )?;
    let mut key_array = [0u8; PUBLIC_KEY_LEN];
    key_array.copy_from_slice(&key_bytes);
    let verifying_key = VerifyingKey::from_bytes(&key_array)
        .map_err(|err| update_error(format!("clé publique Ed25519 invalide : {err}")))?;

    let signature_bytes =
        decode_fixed_hex(signature_hex.trim(), SIGNATURE_LEN, "signature Ed25519")?;
    let mut signature_array = [0u8; SIGNATURE_LEN];
    signature_array.copy_from_slice(&signature_bytes);
    let signature = Signature::from_bytes(&signature_array);

    verifying_key
        .verify_strict(manifest_bytes, &signature)
        .map_err(|_| {
            update_error(
                "signature Ed25519 invalide : le manifeste ne correspond pas à la signature",
            )
        })?;

    tracing::debug!("signature Ed25519 du manifeste vérifiée");
    Ok(())
}

/// Vérifie qu'un fichier a bien l'empreinte SHA-256 attendue.
///
/// Le fichier est lu en flux (jamais chargé entièrement en mémoire) et la
/// comparaison se fait sur des octets, en minuscules, sans sortie anticipée
/// dépendant du contenu.
pub fn verify_digest(path: &Path, expected_sha256: &str) -> IhResult<()> {
    let expected = normalize_sha256(expected_sha256)?;
    let expected_bytes =
        hex::decode(&expected).map_err(|_| update_error("empreinte SHA-256 attendue illisible"))?;

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; READ_BUFFER_LEN];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual = hasher.finalize();
    if !constant_time_eq(actual.as_slice(), &expected_bytes) {
        return Err(update_error(format!(
            "empreinte SHA-256 non conforme pour {}",
            path.display()
        )));
    }

    tracing::debug!(path = %path.display(), "empreinte SHA-256 vérifiée");
    Ok(())
}

/// Normalise une empreinte SHA-256 hexadécimale (64 caractères) en minuscules.
pub(crate) fn normalize_sha256(raw: &str) -> IhResult<String> {
    let trimmed = raw.trim();
    if trimmed.len() != SHA256_HEX_LEN || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(update_error(
            "empreinte SHA-256 invalide (64 caractères hexadécimaux attendus)",
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// Comparaison d'octets à temps constant (la longueur n'est pas secrète).
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left_byte, right_byte) in left.iter().zip(right.iter()) {
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

fn decode_fixed_hex(value: &str, expected_len: usize, label: &str) -> IhResult<Vec<u8>> {
    if value.is_empty() {
        return Err(update_error(format!("{label} vide")));
    }
    let bytes =
        hex::decode(value).map_err(|_| update_error(format!("{label} : hexadécimal invalide")))?;
    if bytes.len() != expected_len {
        return Err(update_error(format!(
            "{label} : {expected_len} octets attendus, {} obtenus",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn update_error(message: impl Into<String>) -> IhError {
    IhError::Update(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn manifest_bytes() -> Vec<u8> {
        serde_json::json!({
            "schema": 1,
            "version": "0.2.0",
            "channel": "stable",
            "notes": "notes",
            "platforms": {
                "linux-x86_64-appimage": {
                    "url": "https://releases.example.com/IH.AppImage",
                    "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                    "kind": "appimage"
                }
            }
        })
        .to_string()
        .into_bytes()
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    #[test]
    fn signature_valide_acceptee() {
        let key = signing_key();
        let bytes = manifest_bytes();
        let signature = key.sign(&bytes);
        verify_signature(
            &bytes,
            &hex::encode(signature.to_bytes()),
            &hex::encode(key.verifying_key().to_bytes()),
        )
        .unwrap();
    }

    #[test]
    fn paire_de_cles_generee_aleatoirement() {
        // Exerce la génération de clés OsRng demandée par le contrat.
        let key = SigningKey::generate(&mut OsRng);
        let bytes = manifest_bytes();
        let signature = key.sign(&bytes);
        verify_signature(
            &bytes,
            &hex::encode(signature.to_bytes()),
            &hex::encode(key.verifying_key().to_bytes()),
        )
        .unwrap();
    }

    #[test]
    fn manifeste_altere_refuse() {
        let key = signing_key();
        let signature = key.sign(&manifest_bytes());
        let altered = String::from_utf8(manifest_bytes())
            .unwrap()
            .replace("0.2.0", "0.2.1")
            .into_bytes();

        let err = verify_signature(
            &altered,
            &hex::encode(signature.to_bytes()),
            &hex::encode(key.verifying_key().to_bytes()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("signature"));
    }

    #[test]
    fn mauvaise_cle_refusee() {
        let key = signing_key();
        let other = SigningKey::from_bytes(&[7u8; 32]);
        let bytes = manifest_bytes();
        let signature = key.sign(&bytes);

        let err = verify_signature(
            &bytes,
            &hex::encode(signature.to_bytes()),
            &hex::encode(other.verifying_key().to_bytes()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("signature"));
    }

    #[test]
    fn signature_hex_invalide_refusee() {
        let key = signing_key();
        let bytes = manifest_bytes();
        let public_key = hex::encode(key.verifying_key().to_bytes());

        assert!(verify_signature(&bytes, "pas-hexadecimal", &public_key).is_err());
        assert!(verify_signature(&bytes, &"a".repeat(126), &public_key).is_err());
        assert!(verify_signature(&bytes, "", &public_key).is_err());
    }

    #[test]
    fn cle_publique_invalide_refusee() {
        let key = signing_key();
        let bytes = manifest_bytes();
        let signature = hex::encode(key.sign(&bytes).to_bytes());

        assert!(verify_signature(&bytes, &signature, "zz").is_err());
        assert!(verify_signature(&bytes, &signature, &"a".repeat(62)).is_err());
        assert!(verify_signature(&bytes, &signature, "").is_err());
    }

    #[test]
    fn empreinte_conforme_acceptee() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artefact.bin");
        std::fs::write(&path, b"abc").unwrap();
        verify_digest(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        // Les majuscules sont acceptées car la comparaison normalise.
        verify_digest(
            &path,
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        )
        .unwrap();
    }

    #[test]
    fn contenu_modifie_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artefact.bin");
        std::fs::write(&path, b"abc").unwrap();
        std::fs::write(&path, b"abd").unwrap();
        let err = verify_digest(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap_err();
        assert!(err.to_string().contains("SHA-256"));
    }

    #[test]
    fn empreinte_attendue_malformee_refusee() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artefact.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert!(verify_digest(&path, "abc").is_err());
        assert!(verify_digest(&path, &"z".repeat(64)).is_err());
    }

    #[test]
    fn fichier_absent_refuse() {
        assert!(
            verify_digest(Path::new("/tmp/ih-update-inexistant-xyz"), &"a".repeat(64)).is_err()
        );
    }
}
