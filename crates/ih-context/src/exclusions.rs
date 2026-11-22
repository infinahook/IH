//! Exclusions de chemins du contexte projet.
//!
//! Les motifs par défaut couvrent les secrets les plus courants, les dossiers
//! techniques et les artefacts de compilation. Un motif fourni par
//! l'utilisateur s'ajoute à cette liste.
//!
//! Règles de correspondance (chemin normalisé en `/`, casse ignorée) :
//! - motif finissant par `/` : correspond à une séquence de composants de
//!   dossier (`node_modules/` matche `a/node_modules/x.js`) ;
//! - motif contenant `*` (ou `?`) : glob simple appliqué au **nom de fichier**
//!   (`*.pem`, `.env.*`) ;
//! - motif contenant `/` : séquence de composants n'importe où dans le chemin ;
//! - sinon : correspond à un composant complet (`id_rsa`, `.env`).

/// Motifs d'exclusion appliqués à tout contexte, dans cet ordre.
const DEFAULT_EXCLUSIONS: [&str; 18] = [
    ".env",
    ".env.*",
    "id_rsa",
    "id_ed25519",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    ".netrc",
    ".git-credentials",
    ".ssh/",
    "node_modules/",
    "target/",
    ".git/",
    "dist/",
    "build/",
    ".venv/",
    "__pycache__/",
];

/// Liste des motifs d'exclusion par défaut.
pub fn default_exclusions() -> Vec<String> {
    DEFAULT_EXCLUSIONS
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect()
}

/// Vrai si le chemin (relatif) correspond à une exclusion par défaut ou
/// fournie. Renvoie la raison sous la forme `exclu: <motif>`.
pub fn exclusion_reason(rel_path: &str, extra: &[String]) -> Option<String> {
    let path = normalize(rel_path);
    if path.is_empty() {
        return None;
    }

    for pattern in DEFAULT_EXCLUSIONS
        .iter()
        .copied()
        .chain(extra.iter().map(String::as_str))
    {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        if pattern_matches(&path, pattern) {
            return Some(format!("exclu: {pattern}"));
        }
    }
    None
}

/// Normalise un chemin pour la correspondance : séparateurs `/`, sans `./`
/// initial ni `/` final, en minuscules.
fn normalize(path: &str) -> String {
    let unified = path.trim().replace('\\', "/");
    let trimmed = unified.trim_start_matches("./").trim_end_matches('/');
    trimmed.to_lowercase()
}

fn pattern_matches(path: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().replace('\\', "/").to_lowercase();
    let pattern = pattern.as_str();

    if pattern.ends_with('/') {
        let directory = pattern.trim_end_matches('/');
        !directory.is_empty() && contains_components(path, directory)
    } else if pattern.contains('*') || pattern.contains('?') {
        let name = path.rsplit('/').next().unwrap_or(path);
        glob_match(pattern, name)
    } else if pattern.contains('/') {
        contains_components(path, pattern)
    } else {
        path.split('/').any(|component| component == pattern)
    }
}

/// Vrai si `needle` apparaît comme séquence complète de composants dans
/// `path` (pas de correspondance au milieu d'un composant).
fn contains_components(path: &str, needle: &str) -> bool {
    let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let needle: Vec<&str> = needle.split('/').filter(|part| !part.is_empty()).collect();
    if needle.is_empty() || needle.len() > path.len() {
        return false;
    }
    path.windows(needle.len()).any(|window| window == needle)
}

/// Glob simple (`*` = toute suite, `?` = un caractère), insensible à la casse
/// car les deux entrées sont déjà en minuscules.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut pattern_index, mut text_index) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut resume_from = 0usize;

    while text_index < text.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == text[text_index])
        {
            pattern_index += 1;
            text_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star = Some(pattern_index);
            pattern_index += 1;
            resume_from = text_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            resume_from += 1;
            text_index = resume_from;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_simple() {
        assert!(glob_match("*.pem", "a.pem"));
        assert!(glob_match(".env.*", ".env.local"));
        assert!(glob_match("*.key", "x.key"));
        assert!(!glob_match("*.pem", "a.pem.bak"));
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("*", "n'importe-quoi"));
    }

    #[test]
    fn sequences_de_composants() {
        assert!(contains_components("a/node_modules/x.js", "node_modules"));
        assert!(contains_components("node_modules/x.js", "node_modules"));
        assert!(!contains_components("my_node_modules/x.js", "node_modules"));
        assert!(contains_components("a/secrets/id_rsa", "secrets/id_rsa"));
        assert!(!contains_components("a/mysecrets/id_rsa", "secrets/id_rsa"));
    }
}
