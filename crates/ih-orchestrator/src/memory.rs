//! Injection de la mémoire persistante dans les prompts.
//!
//! La mémoire est un contexte borné : on ne peut pas laisser un projet
//! accumuler des milliers d'entrées et saturer la fenêtre du modèle.

use ih_core::IhResult;

/// Budget de contexte mémoire par défaut (octets) : ~2 000 jetons.
pub const DEFAULT_MEMORY_BUDGET: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySnippet {
    pub content: String,
    pub source: String,
}

/// Construit un bloc de contexte borné, du plus récent au plus ancien.
///
/// Le résultat est vide si aucune entrée ne tient dans le budget, et ne
/// dépasse jamais `budget` octets.
pub fn build_context(snippets: &[MemorySnippet], budget: usize) -> IhResult<String> {
    if budget == 0 {
        return Ok(String::new());
    }
    let header = "Contexte mémoire du projet (informations durables) :\n";
    let mut out = String::with_capacity(budget.min(16 * 1024));
    if header.len() >= budget {
        return Ok(String::new());
    }
    out.push_str(header);

    let mut used = header.len();
    let mut included = 0usize;
    for snippet in snippets {
        let content = snippet.content.trim();
        if content.is_empty() {
            continue;
        }
        let source = if snippet.source.trim().is_empty() {
            "mémoire".to_string()
        } else {
            snippet.source.trim().to_string()
        };
        let line = format!("- [{source}] {content}\n");
        if used + line.len() > budget {
            continue;
        }
        out.push_str(&line);
        used += line.len();
        included += 1;
    }

    if included == 0 {
        return Ok(String::new());
    }
    Ok(out)
}

/// Estimation grossière du nombre de jetons (4 caractères ≈ 1 jeton).
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(content: &str) -> MemorySnippet {
        MemorySnippet {
            content: content.to_string(),
            source: "test".into(),
        }
    }

    #[test]
    fn contexte_vide_sans_entree() {
        assert_eq!(build_context(&[], 1000).unwrap(), "");
        assert_eq!(build_context(&[snippet("   ")], 1000).unwrap(), "");
    }

    #[test]
    fn budget_respecte_strictement() {
        let snippets: Vec<_> = (0..100)
            .map(|i| snippet(&format!("information durable numéro {i} avec du texte")))
            .collect();
        let context = build_context(&snippets, 500).unwrap();
        assert!(
            context.len() <= 500,
            "dépassement: {} octets",
            context.len()
        );
        assert!(context.contains("Contexte mémoire"));
        assert!(context.matches("- [test]").count() >= 1);
    }

    #[test]
    fn budget_trop_petit_pour_l_entete_donne_vide() {
        assert_eq!(build_context(&[snippet("x")], 10).unwrap(), "");
        assert_eq!(build_context(&[snippet("x")], 0).unwrap(), "");
    }

    #[test]
    fn source_par_defaut() {
        let item = MemorySnippet {
            content: "fait".into(),
            source: "  ".into(),
        };
        let context = build_context(&[item], 500).unwrap();
        assert!(context.contains("[mémoire]"));
    }

    #[test]
    fn estimation_de_jetons() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(""), 0);
    }
}
