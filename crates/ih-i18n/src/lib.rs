//! Internationalisation d'IH.
//!
//! Les catalogues vivent dans `locales/` à la racine du dépôt : ce sont les
//! mêmes fichiers que ceux importés par l'interface React, donc une seule
//! source de vérité. Les tests échouent si les clés FR et EN divergent.

use serde_json::Value;
use std::collections::BTreeSet;

pub const FR_CATALOG: &str = include_str!("../../../locales/fr.json");
pub const EN_CATALOG: &str = include_str!("../../../locales/en.json");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    Fr,
    En,
}

impl Locale {
    pub fn code(self) -> &'static str {
        match self {
            Self::Fr => "fr",
            Self::En => "en",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().as_str() {
            "fr" | "fr-fr" | "fr_fr" => Some(Self::Fr),
            "en" | "en-us" | "en_us" | "en-gb" => Some(Self::En),
            _ => None,
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Fr => Self::En,
            Self::En => Self::Fr,
        }
    }
}

/// Traductions résolues pour une locale, avec repli sur le français.
#[derive(Debug, Clone)]
pub struct I18n {
    locale: Locale,
    primary: Value,
    fallback: Value,
}

impl I18n {
    pub fn new(locale: Locale) -> Self {
        let fr: Value = serde_json::from_str(FR_CATALOG).expect("catalogue FR valide");
        let en: Value = serde_json::from_str(EN_CATALOG).expect("catalogue EN valide");
        match locale {
            Locale::Fr => Self {
                locale,
                primary: fr.clone(),
                fallback: fr,
            },
            Locale::En => Self {
                locale,
                primary: en,
                fallback: fr,
            },
        }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// Traduit une clé pointée (`nav.projects`). Repli : locale → fr → clé.
    pub fn t(&self, key: &str) -> String {
        lookup(&self.primary, key)
            .or_else(|| lookup(&self.fallback, key))
            .unwrap_or(key)
            .to_string()
    }

    /// Traduit avec substitution de variables `{name}`.
    pub fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.t(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    pub fn has(&self, key: &str) -> bool {
        lookup(&self.primary, key).is_some()
    }
}

fn lookup<'a>(catalog: &'a Value, key: &str) -> Option<&'a str> {
    let mut current = catalog;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    current.as_str()
}

/// Ensemble des clés pointées d'un catalogue, triées.
pub fn flatten_keys(raw: &str) -> BTreeSet<String> {
    let value: Value = serde_json::from_str(raw).expect("catalogue JSON valide");
    let mut keys = BTreeSet::new();
    collect(&value, String::new(), &mut keys);
    keys
}

fn collect(value: &Value, prefix: String, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect(child, next, out);
            }
        }
        Value::String(_) => {
            out.insert(prefix);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cles_fr_et_en_identiques() {
        let fr = flatten_keys(FR_CATALOG);
        let en = flatten_keys(EN_CATALOG);
        let manquantes_en: Vec<_> = fr.difference(&en).cloned().collect();
        let manquantes_fr: Vec<_> = en.difference(&fr).cloned().collect();
        assert!(
            manquantes_en.is_empty() && manquantes_fr.is_empty(),
            "clés absentes en EN : {manquantes_en:?} ; absentes en FR : {manquantes_fr:?}"
        );
    }

    #[test]
    fn aucune_traduction_vide() {
        for (label, raw) in [("fr", FR_CATALOG), ("en", EN_CATALOG)] {
            let value: Value = serde_json::from_str(raw).unwrap();
            let mut keys = BTreeSet::new();
            collect(&value, String::new(), &mut keys);
            for key in keys {
                let text = lookup(&value, &key).unwrap_or("");
                assert!(!text.trim().is_empty(), "traduction vide {label}.{key}");
            }
        }
    }

    #[test]
    fn traduction_et_repli() {
        let fr = I18n::new(Locale::Fr);
        let en = I18n::new(Locale::En);
        assert_eq!(fr.locale(), Locale::Fr);
        assert_eq!(fr.t("nav.projects"), "Projets");
        assert_eq!(en.t("nav.projects"), "Projects");
        // clé inexistante : la clé est renvoyée telle quelle
        assert_eq!(en.t("nav.inexistant"), "nav.inexistant");
    }

    #[test]
    fn substitution_de_variables() {
        let fr = I18n::new(Locale::Fr);
        let text = fr.t_args("common.agent_count", &[("count", "3")]);
        assert!(text.contains('3'), "texte: {text}");
    }

    #[test]
    fn parsing_des_codes_de_langue() {
        assert_eq!(Locale::parse("fr-FR"), Some(Locale::Fr));
        assert_eq!(Locale::parse("EN_us"), Some(Locale::En));
        assert_eq!(Locale::parse("de"), None);
    }
}
