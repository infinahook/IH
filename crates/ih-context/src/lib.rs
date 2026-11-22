//! Assemblage sûr du contexte projet envoyé à un modèle.
//!
//! Ce crate transforme une liste de chemins relatifs en un bloc de prompt borné,
//! en refusant explicitement ce qui ne doit pas partir : chemins hors du
//! workspace, fichiers exclus (secrets, dépendances, artefacts), binaires,
//! fichiers trop volumineux et contenus ressemblant à des secrets.
//!
//! ## Garanties
//!
//! - Toute lecture passe par [`ih_workspace::ProjectFs`] ([`ProjectFs::resolve`]
//!   puis [`ProjectFs::read_text`]) : `..`, chemins absolus et symlinks sortants
//!   sont refusés avant toute ouverture.
//! - Les erreurs par fichier deviennent des [`SkippedFile`] et n'interrompent
//!   jamais l'assemblage des autres chemins.
//! - Les limites de fichiers, de taille par fichier et de budget total sont
//!   appliquées strictement ; `total_bytes` ne compte que le contenu réellement
//!   inclus.
//! - Aucun accès réseau, aucune opération asynchrone.
//!
//! ## Limites assumées
//!
//! - La troncature à `max_bytes_per_file` peut couper une ligne ; elle respecte
//!   toujours une frontière de caractère UTF-8.
//! - Un secret situé au-delà de la fenêtre lue (au plus 8 Kio, ou la limite par
//!   fichier si elle est plus grande) n'est pas détecté par
//!   [`content_is_sensitive`].
//! - Le parcours de [`list_files`] et de [`match_mentions`] est borné : sur une
//!   arborescence immense, seules les premières entrées triées sont examinées.
//! - La confidentialité par chemin (exclusions) ne peut pas être contournée par
//!   `allow_sensitive` : ce drapeau ne lève que le classement par **contenu**.

#![deny(unsafe_code)]

mod exclusions;
mod mentions;
mod sensitive;

#[cfg(test)]
mod tests;

pub use exclusions::{default_exclusions, exclusion_reason};
pub use mentions::{list_files, match_mentions, MentionCandidate};
pub use sensitive::content_is_sensitive;

use ih_core::{IhError, IhResult};
use ih_workspace::{ProjectFs, MAX_TEXT_FILE_BYTES};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Component, Path};

/// Nombre maximal de fichiers inclus par défaut.
pub const DEFAULT_MAX_FILES: usize = 12;
/// Taille maximale de contenu incluse par fichier, par défaut (64 Kio).
pub const DEFAULT_MAX_BYTES_PER_FILE: usize = 64 * 1024;
/// Budget total de contenu inclus, par défaut (256 Kio).
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 256 * 1024;

/// Fenêtre minimale lue pour détecter un fichier binaire (octet nul).
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
const BUDGET_REASON: &str = "budget de contexte atteint";
const SENSITIVE_REASON: &str = "sensible (confirmation requise)";
const FILE_LIMIT_REASON: &str = "limite de fichiers atteinte";
const TOO_LARGE_REASON: &str = "trop volumineux";
const BINARY_REASON: &str = "binaire";
const EMPTY_PATH_REASON: &str = "chemin vide";
const PROMPT_HEADER: &str = "Contexte projet (fichiers réellement lus) :";

/// Options d'assemblage du contexte.
#[derive(Debug, Clone)]
pub struct ContextOptions {
    pub max_files: usize,
    pub max_bytes_per_file: usize,
    pub max_total_bytes: usize,
    /// Inclure un fichier classé sensible (après confirmation explicite de
    /// l'utilisateur).
    pub allow_sensitive: bool,
    /// Motifs d'exclusion additionnels fournis par la configuration.
    pub extra_exclusions: Vec<String>,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_FILES,
            max_bytes_per_file: DEFAULT_MAX_BYTES_PER_FILE,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            allow_sensitive: false,
            extra_exclusions: Vec::new(),
        }
    }
}

/// Fichier effectivement inclus dans le contexte.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
    /// Octets réellement inclus (longueur UTF-8 du contenu).
    pub bytes: usize,
    pub truncated: bool,
    pub sensitive: bool,
}

/// Fichier écarté, avec une raison explicite destinée à l'interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

/// Contexte prêt à être injecté dans un prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedContext {
    pub files: Vec<ContextFile>,
    pub skipped: Vec<SkippedFile>,
    pub total_bytes: usize,
    /// Bloc texte prêt à être injecté dans le prompt (vide si aucun fichier).
    pub prompt_block: String,
}

/// Assemble le contexte. Chemins relatifs au workspace, dédupliqués, dans
/// l'ordre fourni.
///
/// Une erreur n'est renvoyée que si la racine du projet elle-même est invalide ;
/// tous les problèmes de fichier sont consignés dans `skipped`.
pub fn prepare(
    project_root: &Path,
    paths: &[String],
    options: &ContextOptions,
) -> IhResult<PreparedContext> {
    let project = ProjectFs::new(project_root)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut files: Vec<ContextFile> = Vec::new();
    let mut skipped: Vec<SkippedFile> = Vec::new();
    let mut entries = String::new();
    let mut total_bytes = 0usize;
    let mut budget_exhausted = false;
    let mut files_exhausted = false;

    for raw_path in paths {
        let rel = normalize_input(raw_path);
        if rel.is_empty() {
            skipped.push(SkippedFile {
                path: raw_path.trim().to_string(),
                reason: EMPTY_PATH_REASON.to_string(),
            });
            continue;
        }
        // Déduplication silencieuse : un même chemin demandé deux fois n'est
        // inclus qu'une fois, à sa première position.
        if !seen.insert(rel.clone()) {
            continue;
        }

        if budget_exhausted {
            skipped.push(SkippedFile {
                path: rel,
                reason: BUDGET_REASON.to_string(),
            });
            continue;
        }
        if files_exhausted || files.len() >= options.max_files {
            files_exhausted = true;
            skipped.push(SkippedFile {
                path: rel,
                reason: FILE_LIMIT_REASON.to_string(),
            });
            continue;
        }

        // Validation structurelle (absolus, `..`, symlinks sortants) avant
        // toute inspection du fichier.
        let resolved = match project.resolve(&rel) {
            Ok(resolved) => resolved,
            Err(error) => {
                skipped.push(SkippedFile {
                    path: rel,
                    reason: path_error_reason(&error),
                });
                continue;
            }
        };

        if let Some(reason) = exclusion_reason(&rel, &options.extra_exclusions) {
            skipped.push(SkippedFile { path: rel, reason });
            continue;
        }

        // Défense en profondeur : un symlink interne peut pointer vers un
        // fichier exclu. On applique les exclusions au chemin canonicalisé
        // (la cible), pas seulement au nom demandé.
        let canonical_exclusion = relative_to_root(project.root(), &resolved)
            .filter(|canonical| canonical != &rel)
            .and_then(|canonical| exclusion_reason(&canonical, &options.extra_exclusions));
        if let Some(reason) = canonical_exclusion {
            skipped.push(SkippedFile { path: rel, reason });
            continue;
        }

        let metadata = match std::fs::metadata(&resolved) {
            Ok(metadata) => metadata,
            Err(error) => {
                skipped.push(SkippedFile {
                    path: rel,
                    reason: format!("illisible: {error}"),
                });
                continue;
            }
        };
        if !metadata.is_file() {
            skipped.push(SkippedFile {
                path: rel,
                reason: "illisible: ce n'est pas un fichier".to_string(),
            });
            continue;
        }
        if metadata.len() > MAX_TEXT_FILE_BYTES as u64 {
            skipped.push(SkippedFile {
                path: rel,
                reason: TOO_LARGE_REASON.to_string(),
            });
            continue;
        }

        // On lit au moins 8 Kio (détection binaire) et jusqu'à la limite par
        // fichier + 4 octets, pour pouvoir couper proprement un caractère
        // UTF-8 multi-octets à la frontière.
        let read_limit = options
            .max_bytes_per_file
            .saturating_add(4)
            .clamp(BINARY_SNIFF_BYTES, MAX_TEXT_FILE_BYTES);
        let read = match project.read_text(&rel, read_limit) {
            Ok(read) => read,
            Err(error) => {
                skipped.push(SkippedFile {
                    path: rel,
                    reason: read_error_reason(&error),
                });
                continue;
            }
        };

        // Détection sur la fenêtre réellement lue, avant troncature : un secret
        // présent juste après la limite par fichier reste ainsi signalé.
        let sensitive = content_is_sensitive(&read.content);
        if sensitive.is_some() && !options.allow_sensitive {
            skipped.push(SkippedFile {
                path: rel,
                reason: SENSITIVE_REASON.to_string(),
            });
            continue;
        }

        let mut content = read.content;
        if content.len() > options.max_bytes_per_file {
            let mut limit = options.max_bytes_per_file.min(content.len());
            while limit > 0 && !content.is_char_boundary(limit) {
                limit -= 1;
            }
            content.truncate(limit);
        }
        let bytes = content.len();
        let truncated = read.truncated || read.size > bytes as u64;

        if total_bytes.saturating_add(bytes) > options.max_total_bytes {
            // Budget épuisé : le fichier qui ne tient pas et tous les suivants
            // sont écartés explicitement.
            budget_exhausted = true;
            skipped.push(SkippedFile {
                path: rel,
                reason: BUDGET_REASON.to_string(),
            });
            continue;
        }

        total_bytes = total_bytes.saturating_add(bytes);
        entries.push_str(&format_prompt_entry(&rel, &content, truncated, read.size));
        files.push(ContextFile {
            path: rel,
            content,
            bytes,
            truncated,
            sensitive: sensitive.is_some(),
        });
    }

    let prompt_block = if files.is_empty() {
        String::new()
    } else {
        format!("{PROMPT_HEADER}\n{entries}")
    };

    Ok(PreparedContext {
        files,
        skipped,
        total_bytes,
        prompt_block,
    })
}

/// Normalise un chemin fourni : composants `.` retirés, séparateurs du système,
/// `..` conservé pour que la prison de chemins le refuse.
fn normalize_input(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if Path::new(raw).is_absolute() {
        return raw.to_string();
    }

    let mut parts: Vec<String> = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => parts.push("..".to_string()),
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::Prefix(_) => return raw.to_string(),
        }
    }
    parts.join("/")
}

/// Chemin relatif (séparateurs `/`) d'un chemin canonique sous la racine.
fn relative_to_root(root: &Path, resolved: &Path) -> Option<String> {
    let relative = resolved.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn path_error_reason(error: &IhError) -> String {
    match error {
        IhError::PathEscape(_) => "hors workspace".to_string(),
        other => format!("illisible: {other}"),
    }
}

fn read_error_reason(error: &IhError) -> String {
    match error {
        IhError::PathEscape(_) => "hors workspace".to_string(),
        IhError::Invalid(message) if message.contains("binaire") => BINARY_REASON.to_string(),
        other => format!("illisible: {other}"),
    }
}

fn format_prompt_entry(path: &str, content: &str, truncated: bool, original_size: u64) -> String {
    let mut entry = String::new();
    if truncated {
        entry.push_str(&format!(
            "--- {path} (tronqué, {} lus sur {}) ---\n",
            display_size(content.len() as u64),
            display_size(original_size)
        ));
    } else {
        entry.push_str(&format!(
            "--- {path} ({}) ---\n",
            display_size(content.len() as u64)
        ));
    }
    entry.push_str(content);
    if !content.ends_with('\n') {
        entry.push('\n');
    }
    entry
}

/// Taille lisible et stable : octets groupés sous 10 Kio, puis Kio/Mio.
fn display_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;

    if bytes < 10 * KIB {
        format!("{} octets", group_thousands(bytes))
    } else if bytes < MIB {
        if bytes % KIB == 0 {
            format!("{} Kio", bytes / KIB)
        } else {
            format!("{:.1} Kio", bytes as f64 / KIB as f64)
        }
    } else if bytes % MIB == 0 {
        format!("{} Mio", bytes / MIB)
    } else {
        format!("{:.1} Mio", bytes as f64 / MIB as f64)
    }
}

fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    grouped
}
