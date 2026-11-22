//! Redaction des secrets avant écriture dans les logs, la base ou l'IPC.
//!
//! Le principe est volontairement pessimiste : on masque tout ce qui ressemble
//! à une clé, même si le contexte est inconnu.

use regex::Regex;
use std::sync::OnceLock;

const MASK: &str = "[masqué]";

struct Patterns {
    bearer: Regex,
    known_prefix: Regex,
    api_key_assign: Regex,
    url_key_param: Regex,
    aws_key: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        bearer: Regex::new(r"(?i)\b(bearer|token)\s+[A-Za-z0-9._\-+/=]{8,}").unwrap(),
        known_prefix: Regex::new(
            r"(?x)
              \b(?: sk-[A-Za-z0-9_\-]{12,}
                 | ghp_[A-Za-z0-9]{16,}
                 | gho_[A-Za-z0-9]{16,}
                 | ghs_[A-Za-z0-9]{16,}
                 | github_pat_[A-Za-z0-9_]{20,}
                 | xox[baprs]-[A-Za-z0-9\-]{10,}
                 | AIza[A-Za-z0-9_\-]{30,}
                 | eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}
                 )
            ",
        )
        .unwrap(),
        api_key_assign: Regex::new(
            r#"(?i)\b(api[_-]?key|secret|password|passwd|access[_-]?token|refresh[_-]?token)\s*[=:]\s*["']?[^\s"'&]{6,}"#,
        )
        .unwrap(),
        url_key_param: Regex::new(r"(?i)([?&](?:api[_-]?key|key|token|access_token)=)[^&\s]{6,}").unwrap(),
        aws_key: Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(),
    })
}

/// Masque les secrets détectables dans un texte libre.
pub fn redact(input: &str) -> String {
    let p = patterns();
    let step1 = p.known_prefix.replace_all(input, MASK);
    let step2 = p.aws_key.replace_all(&step1, MASK);
    let step3 = p.bearer.replace_all(&step2, |caps: &regex::Captures| {
        format!("{} {}", &caps[1], MASK)
    });
    let step4 = p
        .api_key_assign
        .replace_all(&step3, |caps: &regex::Captures| {
            format!("{}={}", &caps[1], MASK)
        });
    let step5 = p
        .url_key_param
        .replace_all(&step4, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], MASK)
        });
    step5.into_owned()
}

/// Redacte récursivement les chaînes d'une valeur JSON (logs structurés).
pub fn redact_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(redact(s)),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_json).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let lowered = k.to_ascii_lowercase();
                    let sensitive = lowered.contains("key")
                        || lowered.contains("token")
                        || lowered.contains("secret")
                        || lowered.contains("password");
                    let value = if sensitive {
                        serde_json::Value::String(MASK.into())
                    } else {
                        redact_json(v)
                    };
                    (k.clone(), value)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masque_les_cles_connues() {
        let input = "clé sk-abcdefghijklmnopqrstuvwxyz et ghp_0123456789abcdefghij";
        let out = redact(input);
        assert!(!out.contains("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!out.contains("ghp_0123456789abcdefghij"));
        assert!(out.contains(MASK));
    }

    #[test]
    fn masque_bearer_et_affectations() {
        let out = redact("Authorization: Bearer abcdefgh12345678\napi_key=secretvalue123");
        assert!(!out.contains("abcdefgh12345678"));
        assert!(!out.contains("secretvalue123"));
    }

    #[test]
    fn masque_les_parametres_d_url() {
        let out = redact("https://example.org/v1?api_key=abcdef123456&x=1");
        assert!(!out.contains("abcdef123456"));
        assert!(
            out.contains("x=1"),
            "les paramètres non sensibles restent: {out}"
        );
    }

    #[test]
    fn conserve_le_texte_ordinaire() {
        let input = "cargo build --release dans /home/user/projet";
        assert_eq!(redact(input), input);
    }

    #[test]
    fn redact_json_masque_les_champs_sensibles() {
        let value = serde_json::json!({
            "api_key": "abcdef123456",
            "nested": { "token": "xyz123456" },
            "message": "ok"
        });
        let out = redact_json(&value);
        assert_eq!(out["api_key"], MASK);
        assert_eq!(out["nested"]["token"], MASK);
        assert_eq!(out["message"], "ok");
    }
}
