//! Détection heuristique de secrets dans un contenu textuel.
//!
//! L'objectif n'est pas d'être exhaustif (impossible sans analyse sémantique)
//! mais d'éviter qu'un fichier manifestement porteur de secret parte dans un
//! prompt sans confirmation. Les motifs sont statiques et compilés une seule
//! fois ; ils s'appliquent au contenu effectivement lu (borné par
//! `ih-workspace`).
//!
//! Limite assumée : un secret situé au-delà de la fenêtre de lecture, ou
//! encodé d'une manière non reconnue, n'est pas détecté.

use regex::Regex;
use std::sync::LazyLock;

/// Motifs (compilés une fois) et raison lisible associée.
static SENSITIVE_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    let patterns: [(&str, &'static str); 9] = [
        (
            r"(?i)-----BEGIN(?: [A-Z0-9]+)* PRIVATE KEY-----",
            "bloc de clé privée",
        ),
        (
            r#"(?i)\b(?:api[_-]?key|secret[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret)\b\s*[:=]\s*["']?[A-Za-z0-9_\-.]{3,}"#,
            "clé ou jeton d'API",
        ),
        (
            r#"(?i)\b(?:password|passwd|pwd)\b\s*[:=]\s*["']?[^\s"']{4,}"#,
            "mot de passe",
        ),
        (r"\bsk-(?:[A-Za-z0-9_-]{8,}|\.{3,})", "jeton d'API"),
        (r"\bgh[pousr]_[A-Za-z0-9]{20,}", "jeton GitHub"),
        (r"\bAKIA[0-9A-Z]{16}\b", "clé AWS"),
        (r"\bAIza[0-9A-Za-z_-]{35}\b", "clé Google"),
        (r"(?i)\bBearer\s+[A-Za-z0-9._\-]{20,}", "jeton Bearer"),
        (r"\bxox[baprs]-[A-Za-z0-9-]{10,}", "jeton Slack"),
    ];

    patterns
        .into_iter()
        .map(|(pattern, reason)| {
            (
                Regex::new(pattern).expect("motif de détection sensible valide"),
                reason,
            )
        })
        .collect()
});

/// Heuristique : le contenu ressemble-t-il à un secret (clé privée, jeton,
/// mot de passe) ? Renvoie une raison lisible si c'est le cas.
pub fn content_is_sensitive(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    SENSITIVE_PATTERNS
        .iter()
        .find(|(pattern, _)| pattern.is_match(content))
        .map(|(_, reason)| (*reason).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecte_les_formes_courantes() {
        assert!(content_is_sensitive(
            "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n-----END PRIVATE KEY-----"
        )
        .is_some());
        assert!(content_is_sensitive("api_key = \"sk-live-abc123456789\"").is_some());
        assert!(content_is_sensitive("token: sk-...").is_some());
        assert!(content_is_sensitive("token: sk-abcdefghijklmnopqrstuvwxyz012345").is_some());
        assert!(content_is_sensitive("password = \"hunter2hunter2\"").is_some());
        assert!(content_is_sensitive("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE").is_some());
        assert!(
            content_is_sensitive("Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345")
                .is_some()
        );
    }

    #[test]
    fn texte_ordinaire_non_sensible() {
        let text = "fn main() {\n    println!(\"bonjour\");\n}\n\n# Documentation\n\
                    Le mot password est décrit ici sans valeur affectée.\n";
        assert!(content_is_sensitive(text).is_none());
        assert!(content_is_sensitive("").is_none());
    }
}
