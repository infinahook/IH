//! Proposition d'appel d'outil MCP émise par le modèle dans sa réponse.
//!
//! Le modèle ne peut pas exécuter d'outil : l'interface et le noyau exigent une
//! approbation à usage unique. Cette convention est un **dialogue** en texte :
//! le modèle reçoit la liste des outils activés dans son prompt système et,
//! s'il juge un outil utile, il termine sa réponse par un bloc balisé dont le
//! langage est `ih-mcp-call` et dont le contenu est un objet JSON
//! `{"server_id": …, "tool": …, "arguments": {...}}` (voir le test
//! `proposition_valide_extraite` pour la forme exacte).
//!
//! IH n'exécute **jamais** le bloc tel quel : il propose l'appel à
//! l'utilisateur, qui l'approuve explicitement (approbation liée à l'empreinte
//! des paramètres), puis l'exécution passe par le point d'entrée unique MCP.

use serde::Serialize;
use serde_json::Value;

/// Nombre maximal de propositions retenues dans une réponse.
pub const MAX_PROPOSALS: usize = 3;
/// Taille maximale de la réponse analysée (le reste est ignoré).
pub const MAX_ANALYSED_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct McpToolProposal {
    pub server_id: String,
    pub tool: String,
    pub arguments: Value,
}

/// Extrait les propositions valides d'une réponse de modèle.
///
/// `enabled` est la liste des couples (serveur, outil) réellement activés :
/// toute proposition citant un outil inconnu ou désactivé est ignorée. Les
/// blocs malformés, les arguments non objets et les propositions en double
/// sont écartés. Aucune erreur n'est renvoyée : une réponse illisible ne doit
/// jamais faire échouer un tour de conversation.
pub fn extract_proposals(text: &str, enabled: &[(String, String)]) -> Vec<McpToolProposal> {
    let truncated = if text.len() > MAX_ANALYSED_BYTES {
        let mut cut = MAX_ANALYSED_BYTES;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        &text[..cut]
    } else {
        text
    };

    let mut proposals: Vec<McpToolProposal> = Vec::new();
    let mut cursor = 0usize;
    while let Some(start) = truncated[cursor..].find("```") {
        let fence_start = cursor + start + 3;
        let Some(info_end) = truncated[fence_start..].find('\n') else {
            break;
        };
        let info = truncated[fence_start..fence_start + info_end].trim();
        let body_start = fence_start + info_end + 1;
        let Some(end) = truncated[body_start..].find("```") else {
            break;
        };
        let body = &truncated[body_start..body_start + end];
        cursor = body_start + end + 3;

        if info != "ih-mcp-call" {
            continue;
        }
        if proposals.len() >= MAX_PROPOSALS {
            break;
        }
        if let Some(proposal) = parse_block(body, enabled) {
            if !proposals
                .iter()
                .any(|p| p.server_id == proposal.server_id && p.tool == proposal.tool)
            {
                proposals.push(proposal);
            }
        }
    }
    proposals
}

fn parse_block(body: &str, enabled: &[(String, String)]) -> Option<McpToolProposal> {
    let value: Value = serde_json::from_str(body.trim()).ok()?;
    let server_id = value.get("server_id")?.as_str()?.trim().to_string();
    let tool = value.get("tool")?.as_str()?.trim().to_string();
    if server_id.is_empty() || tool.is_empty() {
        return None;
    }
    if !enabled
        .iter()
        .any(|(server, name)| *server == server_id && *name == tool)
    {
        return None;
    }
    let arguments = match value.get("arguments") {
        Some(Value::Object(_)) => value.get("arguments").cloned().unwrap_or(Value::Null),
        _ => serde_json::json!({}),
    };
    Some(McpToolProposal {
        server_id,
        tool,
        arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> Vec<(String, String)> {
        vec![("srv".into(), "echo".into()), ("srv".into(), "add".into())]
    }

    #[test]
    fn proposition_valide_extraite() {
        let text = "Voici l'appel :\n\n```ih-mcp-call\n{\"server_id\":\"srv\",\"tool\":\"echo\",\"arguments\":{\"text\":\"bonjour\"}}\n```\n";
        let proposals = extract_proposals(text, &enabled());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].server_id, "srv");
        assert_eq!(proposals[0].tool, "echo");
        assert_eq!(proposals[0].arguments["text"], "bonjour");
    }

    #[test]
    fn outil_non_active_ignore() {
        let text = "```ih-mcp-call\n{\"server_id\":\"srv\",\"tool\":\"delete_everything\",\"arguments\":{}}\n```";
        assert!(extract_proposals(text, &enabled()).is_empty());
    }

    #[test]
    fn json_invalide_ignore() {
        let text = "```ih-mcp-call\n{ceci n'est pas du JSON}\n```";
        assert!(extract_proposals(text, &enabled()).is_empty());
    }

    #[test]
    fn autre_balise_ignoree() {
        let text = "```json\n{\"server_id\":\"srv\",\"tool\":\"echo\",\"arguments\":{}}\n```";
        assert!(extract_proposals(text, &enabled()).is_empty());
    }

    #[test]
    fn arguments_non_objet_normalises() {
        let text =
            "```ih-mcp-call\n{\"server_id\":\"srv\",\"tool\":\"add\",\"arguments\":[1,2]}\n```";
        let proposals = extract_proposals(text, &enabled());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn doublons_et_limite_de_propositions() {
        let mut text = String::new();
        for _ in 0..5 {
            text.push_str(
                "```ih-mcp-call\n{\"server_id\":\"srv\",\"tool\":\"echo\",\"arguments\":{}}\n```\n",
            );
        }
        text.push_str("```ih-mcp-call\n{\"server_id\":\"srv\",\"tool\":\"add\",\"arguments\":{\"a\":1,\"b\":2}}\n```\n");
        let proposals = extract_proposals(&text, &enabled());
        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].tool, "echo");
        assert_eq!(proposals[1].tool, "add");
    }

    #[test]
    fn absence_de_proposition() {
        assert!(extract_proposals("Réponse normale sans outil.", &enabled()).is_empty());
        assert!(extract_proposals("```ih-mcp-call\n```", &enabled()).is_empty());
    }
}
