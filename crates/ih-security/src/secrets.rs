//! Stockage des secrets.
//!
//! Le trousseau du système (Secret Service / keyring) est le seul backend de
//! production. Il n'existe **aucun repli en clair** : si le trousseau est
//! indisponible, l'utilisateur doit le déverrouiller, et l'application refuse
//! d'enregistrer la clé plutôt que de la laisser sur disque.

use ih_core::{IhError, IhResult};
use serde::{Deserialize, Serialize};

/// Compte utilisé dans le trousseau pour un fournisseur donné.
pub fn provider_account(provider_id: &str) -> String {
    format!("provider:{provider_id}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SecretStoreStatus {
    Available { backend: String },
    Unavailable { reason: String },
}

pub trait SecretStore: Send + Sync {
    fn set(&self, account: &str, secret: &str) -> IhResult<()>;
    fn get(&self, account: &str) -> IhResult<Option<String>>;
    fn delete(&self, account: &str) -> IhResult<()>;
    fn status(&self) -> SecretStoreStatus;
}

/// Backend de production : trousseau du système.
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, account: &str) -> IhResult<keyring::Entry> {
        keyring::Entry::new(&self.service, account).map_err(|e| {
            IhError::Denied(format!(
                "trousseau indisponible (est-il déverrouillé ?) : {e}"
            ))
        })
    }
}

impl SecretStore for KeyringSecretStore {
    fn set(&self, account: &str, secret: &str) -> IhResult<()> {
        if secret.trim().is_empty() {
            return Err(IhError::Invalid("secret vide".into()));
        }
        self.entry(account)?
            .set_password(secret)
            .map_err(|e| IhError::Internal(format!("écriture dans le trousseau impossible : {e}")))
    }

    fn get(&self, account: &str) -> IhResult<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(IhError::Internal(format!(
                "lecture dans le trousseau impossible : {e}"
            ))),
        }
    }

    fn delete(&self, account: &str) -> IhResult<()> {
        match self.entry(account)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(IhError::Internal(format!(
                "suppression dans le trousseau impossible : {e}"
            ))),
        }
    }

    fn status(&self) -> SecretStoreStatus {
        // Aucune lecture du trousseau ici : un `get_password` sur une entrée
        // sonde déclenche le déverrouillage du trousseau système (demande de
        // mot de passe) dès l'ouverture de l'application. La disponibilité se
        // limite donc à vérifier qu'une entrée peut être construite ; le
        // déverrouillage n'est demandé qu'au premier usage réel d'un secret
        // (clé fournisseur, jeton MCP) ou via `probe`.
        match keyring::Entry::new(&self.service, "ih-probe") {
            Ok(_) => SecretStoreStatus::Available {
                backend: "secret-service".into(),
            },
            Err(e) => SecretStoreStatus::Unavailable {
                reason: e.to_string(),
            },
        }
    }
}

impl KeyringSecretStore {
    /// Vérifie l'accès réel au trousseau (lecture d'une entrée sonde).
    ///
    /// Cette opération peut demander le déverrouillage du trousseau : elle est
    /// réservée au test explicite de l'utilisateur, jamais au démarrage.
    pub fn probe(&self) -> SecretStoreStatus {
        match keyring::Entry::new(&self.service, "ih-probe") {
            Ok(entry) => match entry.get_password() {
                Ok(_) | Err(keyring::Error::NoEntry) => SecretStoreStatus::Available {
                    backend: "secret-service".into(),
                },
                Err(e) => SecretStoreStatus::Unavailable {
                    reason: e.to_string(),
                },
            },
            Err(e) => SecretStoreStatus::Unavailable {
                reason: e.to_string(),
            },
        }
    }
}

/// Backend en mémoire, réservé aux tests : il perd tout à la fermeture.
#[derive(Default)]
pub struct MemorySecretStore {
    entries: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn set(&self, account: &str, secret: &str) -> IhResult<()> {
        if secret.trim().is_empty() {
            return Err(IhError::Invalid("secret vide".into()));
        }
        self.entries
            .lock()
            .map_err(|_| IhError::Internal("stockage mémoire empoisonné".into()))?
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, account: &str) -> IhResult<Option<String>> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| IhError::Internal("stockage mémoire empoisonné".into()))?
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> IhResult<()> {
        self.entries
            .lock()
            .map_err(|_| IhError::Internal("stockage mémoire empoisonné".into()))?
            .remove(account);
        Ok(())
    }

    fn status(&self) -> SecretStoreStatus {
        SecretStoreStatus::Available {
            backend: "memory".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compte_par_fournisseur() {
        assert_eq!(provider_account("openai"), "provider:openai");
    }

    #[test]
    fn stockage_memoire_cycle_complet() {
        let store = MemorySecretStore::new();
        assert!(store.get("provider:openai").unwrap().is_none());
        store.set("provider:openai", "sk-test-123").unwrap();
        assert_eq!(
            store.get("provider:openai").unwrap().as_deref(),
            Some("sk-test-123")
        );
        store.delete("provider:openai").unwrap();
        assert!(store.get("provider:openai").unwrap().is_none());
    }

    #[test]
    fn secret_vide_refuse() {
        let store = MemorySecretStore::new();
        assert!(store.set("provider:x", "   ").is_err());
    }

    #[test]
    fn status_du_trousseau_sans_lecture_ni_panne() {
        let store = KeyringSecretStore::new("dev.ih.desktop.test-probe");
        // `status` ne doit pas toucher au trousseau (pas de demande de mot de
        // passe au démarrage) : il se contente de vérifier l'entrée.
        match store.status() {
            SecretStoreStatus::Available { backend } => assert!(!backend.is_empty()),
            SecretStoreStatus::Unavailable { reason } => assert!(!reason.is_empty()),
        }
    }

    #[test]
    fn trousseau_ne_panique_jamais_et_ne_replie_pas_en_clair() {
        let store = KeyringSecretStore::new("dev.ih.desktop.test-probe");
        // Selon l'état du système, la lecture renvoie None ou une erreur
        // explicite : jamais de panique, jamais d'écriture disque.
        match store.get("provider:inutilise") {
            Ok(value) => assert!(value.is_none()),
            Err(e) => assert!(!e.to_string().is_empty()),
        }
        match store.probe() {
            SecretStoreStatus::Available { backend } => assert!(!backend.is_empty()),
            SecretStoreStatus::Unavailable { reason } => assert!(!reason.is_empty()),
        }
    }
}
