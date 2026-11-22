//! Erreurs unifiées du noyau IH.
//!
//! Toute erreur qui traverse une frontière IPC doit rester sérialisable et
//! porter un code stable, afin que l'interface puisse réagir sans analyser un
//! message traduit.

use serde::ser::{Serialize, SerializeStruct, Serializer};

/// Erreur IH, exhaustive pour tous les modules.
#[derive(Debug, thiserror::Error)]
pub enum IhError {
    #[error("configuration invalide : {0}")]
    Config(String),

    #[error("chemin refusé (hors du projet autorisé) : {0}")]
    PathEscape(String),

    #[error("accès refusé : {0}")]
    Denied(String),

    #[error("élément introuvable : {0}")]
    NotFound(String),

    #[error("conflit : {0}")]
    Conflict(String),

    #[error("limite atteinte : {0}")]
    Limit(String),

    #[error("entrée invalide : {0}")]
    Invalid(String),

    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),

    #[error("base de données : {0}")]
    Db(String),

    #[error("réseau : {0}")]
    Network(String),

    #[error("fournisseur IA : {0}")]
    Provider(String),

    #[error("agent : {0}")]
    Agent(String),

    #[error("extension : {0}")]
    Extension(String),

    #[error("mise à jour : {0}")]
    Update(String),

    #[error("fonction non prise en charge : {0}")]
    Unsupported(String),

    #[error("interne : {0}")]
    Internal(String),
}

impl IhError {
    /// Code stable, consommé par le frontend (i18n des erreurs).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Config(_) => "config_invalid",
            Self::PathEscape(_) => "path_escape",
            Self::Denied(_) => "denied",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Limit(_) => "limit_reached",
            Self::Invalid(_) => "invalid_input",
            Self::Io(_) => "io_error",
            Self::Db(_) => "db_error",
            Self::Network(_) => "network_error",
            Self::Provider(_) => "provider_error",
            Self::Agent(_) => "agent_error",
            Self::Extension(_) => "extension_error",
            Self::Update(_) => "update_error",
            Self::Unsupported(_) => "unsupported",
            Self::Internal(_) => "internal_error",
        }
    }

    /// Indique si l'utilisateur peut corriger le problème par une action.
    pub fn is_user_fixable(&self) -> bool {
        matches!(
            self,
            Self::Config(_)
                | Self::Invalid(_)
                | Self::Denied(_)
                | Self::Limit(_)
                | Self::Provider(_)
                | Self::Network(_)
        )
    }
}

impl From<serde_json::Error> for IhError {
    fn from(value: serde_json::Error) -> Self {
        Self::Invalid(value.to_string())
    }
}

impl Serialize for IhError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("IhError", 3)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.serialize_field("fixable", &self.is_user_fixable())?;
        state.end()
    }
}

/// Résultat IH.
pub type IhResult<T> = Result<T, IhError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_sont_stables_et_distincts() {
        let err = IhError::Denied("x".into());
        assert_eq!(err.code(), "denied");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["code"], "denied");
        assert_eq!(json["fixable"], true);
        assert!(json["message"].as_str().unwrap().contains('x'));
    }

    #[test]
    fn erreurs_techniques_ne_sont_pas_fixables_par_l_utilisateur() {
        assert!(!IhError::Internal("boom".into()).is_user_fixable());
        assert!(!IhError::Db("boom".into()).is_user_fixable());
        assert!(IhError::Config("boom".into()).is_user_fixable());
    }
}
