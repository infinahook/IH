//! Complétion des mentions `@fichier` et listing plat des fichiers du projet.
//!
//! Les deux fonctions passent par [`ProjectFs::list_tree`] : les dossiers
//! techniques `.git`, `target` et `node_modules` sont donc déjà écartés par la
//! prison de chemins de `ih-workspace`, et le parcours est borné en profondeur
//! comme en nombre d'entrées.
//!
//! ## Score flou
//!
//! Le classement privilégie, dans l'ordre : chemin exact, nom exact, radical
//! exact (`main` pour `main.rs`), préfixe de chemin, préfixe de nom,
//! sous-chaîne, puis sous-séquence stricte. Une tolérance de faible poids
//! autorise l'omission d'**un** caractère de la requête (utile pour une faute
//! de frappe) ; elle n'est appliquée qu'aux fichiers, jamais aux dossiers.
//! Les bonus de préfixe et de fin de chemin départagent les correspondances.

use crate::exclusions::exclusion_reason;
use ih_core::IhResult;
use ih_workspace::ProjectFs;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Profondeur maximale du parcours des mentions.
const SCAN_DEPTH: usize = 64;
/// Nombre maximal d'entrées examinées pour la complétion `@`.
const MENTION_SCAN_ENTRIES: usize = 20_000;
/// Nombre maximal de candidats renvoyés par [`match_mentions`].
const MAX_MENTION_RESULTS: usize = 200;
/// Nombre maximal d'entrées examinées par [`list_files`].
const LIST_SCAN_ENTRIES: usize = 50_000;
/// Nombre maximal de fichiers renvoyés par [`list_files`].
const MAX_LIST_RESULTS: usize = 10_000;

/// Candidat pour la complétion `@` d'un chemin relatif.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MentionCandidate {
    /// Chemin relatif à la racine, séparateurs `/`.
    pub path: String,
    /// Score flou, plus grand = meilleur.
    pub score: u32,
    pub is_dir: bool,
    /// Vrai si le chemin serait exclu du contexte (renvoyé seulement sur
    /// correspondance exacte, pour que l'interface puisse l'expliquer).
    pub excluded: bool,
}

/// Candidats pour la complétion `@` : recherche floue sur les chemins relatifs.
pub fn match_mentions(
    project_root: &Path,
    query: &str,
    limit: usize,
    extra_exclusions: &[String],
) -> IhResult<Vec<MentionCandidate>> {
    let limit = limit.min(MAX_MENTION_RESULTS);
    let raw_query = query.trim();
    let query = raw_query.to_lowercase();
    if limit == 0 || query.is_empty() {
        return Ok(Vec::new());
    }

    let project = ProjectFs::new(project_root)?;
    let nodes = project.list_tree(SCAN_DEPTH, MENTION_SCAN_ENTRIES)?;

    let mut candidates = Vec::new();
    let mut exact_path_found = false;
    for node in nodes {
        if node.path.is_empty() {
            continue;
        }
        let path = node.path.to_lowercase();
        let basename = path.rsplit('/').next().unwrap_or(&path);
        let excluded = exclusion_reason(&node.path, extra_exclusions).is_some();
        let exact = path == query || basename == query;
        if path == query {
            exact_path_found = true;
        }

        if excluded && !exact {
            continue;
        }

        let score = if excluded {
            // Correspondance exacte d'un chemin exclu : on le renvoie en tête
            // avec le drapeau, sans jamais proposer ses voisins exclus.
            10_000
        } else {
            match score_path(&query, &path, node.is_dir) {
                Some(score) => score,
                None => continue,
            }
        };

        candidates.push(MentionCandidate {
            path: node.path,
            score,
            is_dir: node.is_dir,
            excluded,
        });
    }

    // `ProjectFs::list_tree` élague `.git`, `target` et `node_modules`, et le
    // parcours est borné : une requête exacte sur un chemin absent de la liste
    // (notamment un chemin exclu) doit tout de même être résolue, sans lire
    // son contenu.
    if !exact_path_found {
        if let Some(candidate) = exact_candidate(&project, raw_query, extra_exclusions) {
            if candidates
                .iter()
                .all(|existing| existing.path != candidate.path)
            {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates.truncate(limit);
    Ok(candidates)
}

/// Construit un candidat pour une correspondance exacte absente de
/// l'arborescence élaguée. Seules les métadonnées du chemin résolu par la
/// prison sont consultées, jamais son contenu.
fn exact_candidate(
    project: &ProjectFs,
    raw_query: &str,
    extra_exclusions: &[String],
) -> Option<MentionCandidate> {
    if raw_query.is_empty() {
        return None;
    }
    let resolved = project.resolve(raw_query).ok()?;
    let metadata = std::fs::metadata(&resolved).ok()?;
    let excluded = exclusion_reason(raw_query, extra_exclusions).is_some();
    let path = raw_query
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();
    Some(MentionCandidate {
        path,
        score: 10_000,
        is_dir: metadata.is_dir(),
        excluded,
    })
}

/// Liste plate des fichiers du projet (pour l'explorateur et les mentions),
/// bornée. Les chemins sont triés sans tenir compte de la casse.
pub fn list_files(
    project_root: &Path,
    limit: usize,
    extra_exclusions: &[String],
) -> IhResult<Vec<String>> {
    let limit = limit.min(MAX_LIST_RESULTS);
    if limit == 0 {
        return Ok(Vec::new());
    }

    let project = ProjectFs::new(project_root)?;
    // Marge pour les entrées filtrées (dossiers, exclusions) ; le parcours
    // reste borné même si l'utilisateur demande une limite élevée.
    let scan_limit = limit
        .saturating_mul(40)
        .saturating_add(500)
        .min(LIST_SCAN_ENTRIES);
    let nodes = project.list_tree(SCAN_DEPTH, scan_limit)?;

    let mut files: Vec<String> = nodes
        .into_iter()
        .filter(|node| !node.is_dir)
        .filter(|node| exclusion_reason(&node.path, extra_exclusions).is_none())
        .map(|node| node.path)
        .collect();
    files.sort_by_cached_key(|path| path.to_lowercase());
    files.truncate(limit);
    Ok(files)
}

fn score_path(query: &str, path: &str, is_dir: bool) -> Option<u32> {
    let basename_start = path.rfind('/').map_or(0, |index| index + 1);
    let basename = &path[basename_start..];
    let stem = basename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|stem| !stem.is_empty())
        .unwrap_or(basename);

    let mut score = if path == query {
        10_000
    } else if basename == query {
        9_500
    } else if stem == query {
        9_000
    } else if path.starts_with(query) {
        8_000
    } else if basename.starts_with(query) {
        7_000
    } else if basename.contains(query) {
        6_000
    } else if path.contains(query) {
        5_000
    } else if let Some(score) = subsequence_score(query, path) {
        score
    } else if is_dir {
        return None;
    } else {
        near_subsequence_score(query, path)?
    };

    // Bonus de fin de chemin et de concision : à score principal égal, le
    // chemin le plus court et celui dont le nom se termine par la requête
    // passent devant.
    if path.ends_with(query) {
        score = score.saturating_add(120);
    }
    if basename_starts_with_boundary(query, basename) {
        score = score.saturating_add(80);
    }
    let length = u32::try_from(path.chars().count()).unwrap_or(u32::MAX);
    Some(score.saturating_add(60u32.saturating_sub(length.min(60))))
}

/// Vrai si le nom commence par la requête à une frontière de mot
/// (`main` dans `main.rs`, `main_config.rs`).
fn basename_starts_with_boundary(query: &str, basename: &str) -> bool {
    if !basename.starts_with(query) {
        return false;
    }
    match basename[query.len()..].chars().next() {
        None => true,
        Some(next) => matches!(next, '.' | '_' | '-' | '/' | ' '),
    }
}

fn subsequence_score(query: &str, path: &str) -> Option<u32> {
    let positions = subsequence_positions(query, path)?;
    let matched = u32::try_from(positions.len()).unwrap_or(u32::MAX);
    let first = *positions.first()?;
    let last = *positions.last()?;
    let span = u32::try_from(last - first + 1).unwrap_or(u32::MAX).max(1);
    let density = (matched.saturating_mul(100) / span).min(200);
    let baseline = 1_000u32.saturating_add(matched.saturating_mul(40));

    let mut score = baseline.saturating_add(density);
    let basename_start = path.rfind('/').map_or(0, |index| index + 1);
    if first == basename_start {
        score = score.saturating_add(300);
    } else if first == 0 {
        score = score.saturating_add(200);
    }
    if last + 1 == path.len() {
        score = score.saturating_add(100);
    }
    Some(score)
}

/// Tolérance de frappe : autorise l'omission d'un caractère de la requête.
/// Poids volontairement faible, et jamais utilisé pour les dossiers.
fn near_subsequence_score(query: &str, path: &str) -> Option<u32> {
    let chars: Vec<char> = query.chars().collect();
    if chars.len() < 3 {
        return None;
    }

    let mut best: Option<u32> = None;
    for skipped in 0..chars.len() {
        let candidate: String = chars
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != skipped)
            .map(|(_, character)| *character)
            .collect();
        let Some(positions) = subsequence_positions(&candidate, path) else {
            continue;
        };
        let matched = u32::try_from(positions.len()).unwrap_or(u32::MAX);
        let first = *positions.first()?;
        let last = *positions.last()?;
        let span = u32::try_from(last - first + 1).unwrap_or(u32::MAX).max(1);
        let density = (matched.saturating_mul(100) / span).min(200);
        let mut score = 400u32
            .saturating_add(matched.saturating_mul(20))
            .saturating_add(density);
        let basename_start = path.rfind('/').map_or(0, |index| index + 1);
        if first == basename_start {
            score = score.saturating_add(200);
        } else if first == 0 {
            score = score.saturating_add(100);
        }
        best = Some(best.map_or(score, |current| current.max(score)));
    }
    best
}

/// Positions (octets) des caractères de `query` formant une sous-séquence
/// stricte de `path`, ou `None` si la sous-séquence n'existe pas.
fn subsequence_positions(query: &str, path: &str) -> Option<Vec<usize>> {
    let mut positions = Vec::new();
    let mut characters = path.char_indices();
    for wanted in query.chars() {
        loop {
            let (index, candidate) = characters.next()?;
            if candidate == wanted {
                positions.push(index);
                break;
            }
        }
    }
    Some(positions)
}
