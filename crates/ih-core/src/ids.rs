//! Identifiants typés : évite de confondre un identifiant de projet avec un
//! identifiant de tâche dans les appels internes et dans l'IPC.

use crate::error::{IhError, IhResult};
use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $prefix:literal, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Génère un nouvel identifiant unique.
            pub fn new() -> Self {
                Self(format!("{}-{}", $prefix, uuid::Uuid::new_v4()))
            }

            /// Valide un identifiant fourni par l'extérieur (IPC, base).
            ///
            /// L'alphabet est volontairement strict : un identifiant peut finir
            /// dans un nom de dossier ou de fichier, et une valeur contenant
            /// `/`, `..` ou des caractères de contrôle permettrait de sortir du
            /// répertoire prévu (y compris près d'un `remove_dir_all`).
            pub fn parse(value: impl Into<String>) -> IhResult<Self> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IhError::Invalid(format!("{} vide", $label)));
                }
                if value.len() > 128 {
                    return Err(IhError::Invalid(format!("{} trop long", $label)));
                }
                let safe = value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
                if !safe || value.contains("..") || value.starts_with('.') {
                    return Err(IhError::Invalid(format!(
                        "{} invalide (alphabet [A-Za-z0-9._-] sans « .. ») : {value}",
                        $label
                    )));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

id_type!(ProjectId, "prj", "identifiant de projet");
id_type!(ThreadId, "thr", "identifiant de conversation");
id_type!(SessionId, "ses", "identifiant de session d'agent");
id_type!(TaskId, "tsk", "identifiant de tâche");
id_type!(AgentId, "agt", "identifiant d'agent");
id_type!(MessageId, "msg", "identifiant de message");
id_type!(ServerId, "srv", "identifiant de serveur MCP");
id_type!(ApprovalId, "apr", "identifiant d'approbation");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_identifiants_sont_prefixes_et_uniques() {
        let a = ProjectId::new();
        let b = ProjectId::new();
        assert!(a.as_str().starts_with("prj-"));
        assert_ne!(a, b);
    }

    #[test]
    fn parse_rejette_les_valeurs_vides() {
        assert!(ThreadId::parse("   ").is_err());
        assert_eq!(ThreadId::parse("thr-1").unwrap().as_str(), "thr-1");
    }

    #[test]
    fn parse_refuse_les_identifiants_dangereux_pour_les_chemins() {
        // Traversée de répertoire, séparateurs, caractères de contrôle.
        for bad in [
            "ses-../../etc/passwd",
            "ses-/absolu",
            "ses-..",
            "../ses-x",
            "ses-x/y",
            "ses-x\ty",
            "ses-x y",
        ] {
            assert!(SessionId::parse(bad).is_err(), "{bad} doit être refusé");
        }
        // Les identifiants réellement générés restent acceptés.
        let id = SessionId::new();
        assert_eq!(SessionId::parse(id.as_str()).unwrap(), id);
    }

    #[test]
    fn serialisation_transparente() {
        let id = TaskId::parse("tsk-abc").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"tsk-abc\"");
    }
}
