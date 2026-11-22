//! Normalisation de la sortie des CLI d'agents.
//!
//! OpenCode (`--format json`) émet du NDJSON : `step_start`, `tool_use`,
//! `text`, `step_finish`. IH ne peut pas afficher ces objets bruts : on les
//! transforme en lignes lisibles, sans perdre l'information utile (outil
//! appelé, chemin, commande, texte de l'assistant).
//!
//! Les lignes qui ne sont pas du JSON sont renvoyées telles quelles : un
//! adaptateur qui écrit du texte ne doit jamais être cassé par cette
//! normalisation.
//!
//! Format validé contre OpenCode **v2.3.2** (voir `tests`).

use ih_core::config::AgentKind;
use serde_json::Value;

/// Transforme une ligne de sortie en lignes affichables.
///
/// Retourne un vecteur vide pour les événements purement techniques
/// (`step_start`, `step_finish`) qui n'apportent rien à l'utilisateur.
pub fn normalize_line(kind: &AgentKind, line: &str) -> Vec<String> {
    match kind {
        AgentKind::Opencode => normalize_opencode(line),
        _ => vec![line.to_string()],
    }
}

fn normalize_opencode(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Le NDJSON d'OpenCode n'arrive pas toujours aligné : on tente le parse,
    // sinon on restitue la ligne brute (logs, erreurs non JSON).
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return vec![line.to_string()];
    };
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "text" => value
            .get("part")
            .and_then(|part| part.get("text"))
            .and_then(Value::as_str)
            .map(|text| {
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        "tool_use" => {
            let part = value.get("part");
            let tool = part
                .and_then(|p| p.get("tool"))
                .and_then(Value::as_str)
                .unwrap_or("outil");
            let status = part
                .and_then(|p| p.get("state"))
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let input = part
                .and_then(|p| p.get("state"))
                .and_then(|s| s.get("input"));
            let detail = tool_detail(tool, input);
            let suffix = if status == "error" { " (échec)" } else { "" };
            vec![format!("[{tool}] {detail}{suffix}")]
        }
        "error" => {
            let message = value
                .get("error")
                .and_then(|e| e.get("message").or_else(|| e.get("data")))
                .and_then(Value::as_str)
                .unwrap_or("erreur OpenCode");
            vec![format!("[erreur] {message}")]
        }
        // étape technique : rien à afficher
        "step_start" | "step_finish" => Vec::new(),
        _ => Vec::new(),
    }
}

/// Détail lisible d'un appel d'outil : chemin, commande, motif…
fn tool_detail(tool: &str, input: Option<&Value>) -> String {
    let Some(input) = input else {
        return String::new();
    };
    let pick = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    match tool {
        "shell" | "bash" => pick("command").to_string(),
        "read" | "write" | "edit" | "patch" => {
            let path = pick("path");
            if path.is_empty() {
                pick("filePath").to_string()
            } else {
                path.to_string()
            }
        }
        "grep" | "glob" | "search" => {
            let pattern = pick("pattern");
            let path = pick("path");
            match (pattern.is_empty(), path.is_empty()) {
                (false, false) => format!("{pattern} dans {path}"),
                (false, true) => pattern.to_string(),
                (true, false) => path.to_string(),
                (true, true) => String::new(),
            }
        }
        _ => {
            let path = pick("path");
            if path.is_empty() {
                input.to_string().chars().take(160).collect()
            } else {
                path.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lignes réellement produites par `opencode run --format json` (v2.3.2).
    const REAL_TOOL_READ: &str = r#"{"type":"tool_use","timestamp":1795368482323,"sessionID":"ses_x","part":{"partID":"prt_x","sessionID":"ses_x","messageID":"msg_x","type":"tool","id":"call-1","tool":"read","state":{"status":"completed","input":{"path":"/tmp/oc-probe/a.txt"},"output":"Read file"}}}"#;
    const REAL_TOOL_SHELL: &str = r#"{"type":"tool_use","timestamp":1795368484806,"sessionID":"ses_x","part":{"tool":"shell","state":{"status":"completed","input":{"command":"xxd /tmp/oc-probe/a.txt"},"output":"0000"}}}"#;
    const REAL_TEXT: &str = r#"{"type":"text","timestamp":1795368485000,"sessionID":"ses_x","part":{"type":"text","text":"J'ai ajouté la ligne `bonjour` à la fin de `a.txt`."}}"#;
    const REAL_STEP_START: &str = r#"{"type":"step_start","timestamp":1795368482003,"sessionID":"ses_x","part":{"type":"step-start"}}"#;
    const REAL_STEP_FINISH: &str = r#"{"type":"step_finish","timestamp":1795368482337,"sessionID":"ses_x","part":{"type":"step-finish","reason":"tool-calls"}}"#;

    #[test]
    fn texte_de_l_assistant_est_restitue() {
        let lines = normalize_line(&AgentKind::Opencode, REAL_TEXT);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("bonjour"));
    }

    #[test]
    fn appels_d_outils_sont_lisibles() {
        let read = normalize_line(&AgentKind::Opencode, REAL_TOOL_READ);
        assert_eq!(read, vec!["[read] /tmp/oc-probe/a.txt".to_string()]);

        let shell = normalize_line(&AgentKind::Opencode, REAL_TOOL_SHELL);
        assert_eq!(shell, vec!["[shell] xxd /tmp/oc-probe/a.txt".to_string()]);
    }

    #[test]
    fn etapes_techniques_masquees() {
        assert!(normalize_line(&AgentKind::Opencode, REAL_STEP_START).is_empty());
        assert!(normalize_line(&AgentKind::Opencode, REAL_STEP_FINISH).is_empty());
    }

    #[test]
    fn sortie_non_json_passee_telle_quelle() {
        let raw = "opencode: une erreur réseau";
        assert_eq!(
            normalize_line(&AgentKind::Opencode, raw),
            vec![raw.to_string()]
        );
        assert!(normalize_line(&AgentKind::Opencode, "   ").is_empty());
    }

    #[test]
    fn autres_adaptateurs_inchanges() {
        let raw = "ligne brute";
        for kind in [
            AgentKind::ClaudeCode,
            AgentKind::Codex,
            AgentKind::GeminiCli,
            AgentKind::Custom,
        ] {
            assert_eq!(normalize_line(&kind, raw), vec![raw.to_string()]);
            // Même une ligne JSON reste brute pour les autres CLI.
            assert_eq!(
                normalize_line(&kind, REAL_TEXT),
                vec![REAL_TEXT.to_string()]
            );
        }
    }

    #[test]
    fn erreur_json_est_signalee() {
        let line = r#"{"type":"error","error":{"message":"quota dépassé"}}"#;
        assert_eq!(
            normalize_line(&AgentKind::Opencode, line),
            vec!["[erreur] quota dépassé".to_string()]
        );
    }
}
