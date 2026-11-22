//! Édition sûre des fichiers d'un projet.
//!
//! [`ProjectFs`] est une prison de chemins : `../`, les chemins absolus et les
//! symlinks qui sortent de la racine sont refusés. La résolution canonicalise
//! toujours le plus proche ancêtre existant, puis vérifie que le résultat reste
//! sous la racine du projet.
//!
//! Les fichiers binaires (contenant un octet nul) et les fichiers de plus de
//! [`MAX_TEXT_FILE_BYTES`] sont ignorés par la recherche. La lecture et la
//! recherche sont bornées pour ne jamais charger un projet entier en mémoire.
//!
//! Limite assumée : la vérification est faite avant l'ouverture (TOCTOU) ; un
//! processus hostile pourrait remplacer un dossier par un symlink entre la
//! résolution et l'écriture. L'interface IH n'exécute pas de code non fiable
//! dans le même répertoire.

use ih_core::{IhError, IhResult};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::fs as sysfs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use walkdir::{DirEntry, WalkDir};

use crate::MAX_TEXT_FILE_BYTES;

fn short_hash(hash: &str) -> &str {
    hash.get(0..12).unwrap_or(hash)
}

/// Dossiers techniques exclus de l'exploration et de la recherche.
const IGNORED_DIRS: [&str; 3] = [".git", "target", "node_modules"];

/// Nombre maximal d'entrées renvoyées par [`ProjectFs::list_tree`].
const MAX_TREE_ENTRIES: usize = 100_000;
/// Nombre maximal de résultats renvoyés par [`ProjectFs::search`].
const MAX_SEARCH_RESULTS: usize = 10_000;
/// Longueur maximale d'un aperçu de ligne de recherche (caractères).
const PREVIEW_CHARS: usize = 240;

/// Racine validée d'un projet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFs {
    root: PathBuf,
}

/// Un nœud de l'arborescence projet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNode {
    /// Chemin relatif à la racine, séparateurs `/`.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Contenu textuel borné d'un fichier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    /// Taille réelle du fichier sur disque (octets).
    pub size: u64,
}

/// Occurrence d'un motif dans un fichier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    /// Numéro de ligne, à partir de 1.
    pub line: u32,
    /// Décalage en octets dans la ligne, à partir de 1.
    pub column: u32,
    /// Ligne (tronquée) contenant l'occurrence.
    pub preview: String,
}

/// Métadonnées d'un fichier, utilisées pour détecter les modifications
/// externes et éviter les écrasements silencieux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub path: String,
    pub size: u64,
    /// Horodatage de modification (millisecondes depuis l'époque), si disponible.
    pub modified_ms: Option<u64>,
    /// SHA-256 hexadécimal du contenu.
    pub sha256: String,
    pub is_binary: bool,
}

/// Résultat d'une écriture vérifiée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteOutcome {
    pub bytes: u64,
    pub sha256: String,
    /// Chemin de la sauvegarde créée avant écrasement, si demandée.
    pub backup: Option<String>,
}

/// Vrai si les premiers octets contiennent un zéro (fichier binaire).
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

/// SHA-256 hexadécimal d'un flux d'octets.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

impl ProjectFs {
    /// Ouvre un projet : la racine est canonicalisée une fois pour toutes.
    pub fn new(root: &Path) -> IhResult<Self> {
        let canonical = sysfs::canonicalize(root).map_err(|error| {
            IhError::Invalid(format!(
                "racine de projet invalide ({}): {error}",
                root.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(IhError::Invalid(format!(
                "la racine du projet n'est pas un dossier : {}",
                canonical.display()
            )));
        }
        Ok(Self { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Métadonnées complètes d'un fichier du projet (taille, hash, binarité).
    pub fn metadata(&self, rel_path: &str) -> IhResult<FileMeta> {
        let path = self.resolve(rel_path)?;
        let metadata = sysfs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(IhError::Invalid(format!(
                "ce n'est pas un fichier : {rel_path}"
            )));
        }
        let mut file = sysfs::File::open(&path)?;
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        let mut head = Vec::with_capacity(8192);
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            if head.len() < 8192 {
                let take = (8192 - head.len()).min(read);
                head.extend_from_slice(&buffer[..take]);
            }
            sha2::Digest::update(&mut hasher, &buffer[..read]);
        }
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64);
        Ok(FileMeta {
            path: normalize_rel(Path::new(rel_path)),
            size: metadata.len(),
            modified_ms,
            sha256: hex::encode(sha2::Digest::finalize(hasher)),
            is_binary: looks_binary(&head),
        })
    }

    /// Écriture vérifiée : refuse d'écraser un fichier modifié depuis la
    /// lecture, et sauvegarde l'ancien contenu si `backup_dir` est fourni.
    pub fn write_checked(
        &self,
        rel_path: &str,
        content: &str,
        expected_sha256: Option<&str>,
        backup_dir: Option<&Path>,
    ) -> IhResult<WriteOutcome> {
        let target = self.resolve(rel_path)?;
        let existing = if target.is_file() {
            Some(self.metadata(rel_path)?)
        } else {
            None
        };

        if let (Some(expected), Some(current)) = (expected_sha256, existing.as_ref()) {
            if !expected.eq_ignore_ascii_case(&current.sha256) {
                return Err(IhError::Conflict(format!(
                    "le fichier {rel_path} a changé sur le disque depuis sa lecture \
                     (attendu {}, trouvé {})",
                    short_hash(expected),
                    short_hash(&current.sha256)
                )));
            }
        }

        let mut backup = None;
        if let (Some(dir), Some(current)) = (backup_dir, existing.as_ref()) {
            sysfs::create_dir_all(dir)?;
            let name = Path::new(rel_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "fichier".into());
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let safe: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let backup_path = dir.join(format!(
                "{safe}.{stamp}.{}.bak",
                short_hash(&current.sha256)
            ));
            sysfs::copy(&target, &backup_path)?;
            backup = Some(backup_path.display().to_string());
        }

        let bytes = self.write_text(rel_path, content)?;
        Ok(WriteOutcome {
            bytes,
            sha256: sha256_hex(content.as_bytes()),
            backup,
        })
    }

    /// Arborescence bornée, sans `.git`, `target` ni `node_modules`.
    pub fn list_tree(&self, max_depth: usize, max_entries: usize) -> IhResult<Vec<FileNode>> {
        let max_entries = max_entries.min(MAX_TREE_ENTRIES);
        if max_entries == 0 || max_depth == 0 {
            return Ok(Vec::new());
        }

        let walker = WalkDir::new(&self.root)
            .max_depth(max_depth)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !is_ignored(entry));

        let mut nodes = Vec::new();
        for entry in walker {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::debug!("entrée ignorée pendant list_tree : {error}");
                    continue;
                }
            };
            if entry.depth() == 0 {
                continue;
            }
            let Some(path) = relative_path(&self.root, entry.path()) else {
                continue;
            };
            let metadata = entry.metadata().ok();
            let is_dir = entry.file_type().is_dir();
            let size = if is_dir {
                0
            } else {
                metadata.map(|meta| meta.len()).unwrap_or(0)
            };
            nodes.push(FileNode {
                path,
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                size,
            });
            if nodes.len() >= max_entries {
                break;
            }
        }
        Ok(nodes)
    }

    /// Lit un fichier texte, au plus `max_bytes` octets (plafonné à 2 Mio).
    pub fn read_text(&self, rel_path: &str, max_bytes: usize) -> IhResult<FileContent> {
        if max_bytes == 0 {
            return Err(IhError::Invalid(
                "max_bytes doit être supérieur à zéro".to_string(),
            ));
        }
        let path = self.resolve(rel_path)?;
        let metadata = sysfs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(IhError::Invalid(format!(
                "ce n'est pas un fichier : {rel_path}"
            )));
        }

        let limit = max_bytes.min(MAX_TEXT_FILE_BYTES) as u64;
        let mut bytes = Vec::new();
        sysfs::File::open(&path)?
            .take(limit)
            .read_to_end(&mut bytes)?;
        if bytes.contains(&0) {
            return Err(IhError::Invalid(format!(
                "fichier binaire refusé : {rel_path}"
            )));
        }

        Ok(FileContent {
            path: normalize_rel(Path::new(rel_path)),
            content: String::from_utf8_lossy(&bytes).into_owned(),
            truncated: metadata.len() > bytes.len() as u64,
            size: metadata.len(),
        })
    }

    /// Écrit un fichier texte (dossiers parents créés, remplacement atomique).
    /// Renvoie le nombre d'octets écrits.
    pub fn write_text(&self, rel_path: &str, content: &str) -> IhResult<u64> {
        let target = self.resolve(rel_path)?;
        if target == self.root {
            return Err(IhError::Invalid(
                "impossible d'écrire à la racine du projet".to_string(),
            ));
        }
        if target.is_dir() {
            return Err(IhError::Invalid(format!(
                "un dossier existe à cet emplacement : {rel_path}"
            )));
        }
        let parent = target
            .parent()
            .ok_or_else(|| IhError::Invalid(format!("chemin sans dossier parent : {rel_path}")))?;
        sysfs::create_dir_all(parent)?;

        // Écriture atomique : fichier temporaire dans le même dossier puis
        // renommage, pour ne jamais laisser un fichier à moitié écrit. Le mode
        // du fichier existant est conservé (scripts exécutables).
        let permissions = sysfs::metadata(&target)
            .ok()
            .map(|metadata| metadata.permissions());
        let temp = parent.join(format!(".ih-{}.tmp", Uuid::new_v4()));
        if let Err(error) = sysfs::write(&temp, content) {
            let _ = sysfs::remove_file(&temp);
            return Err(IhError::Io(error));
        }
        if let Some(permissions) = permissions {
            if let Err(error) = sysfs::set_permissions(&temp, permissions) {
                let _ = sysfs::remove_file(&temp);
                return Err(IhError::Io(error));
            }
        }
        if let Err(error) = sysfs::rename(&temp, &target) {
            let _ = sysfs::remove_file(&temp);
            return Err(IhError::Io(error));
        }
        Ok(content.len() as u64)
    }

    /// Recherche texte récursive, insensible à la casse, bornée en résultats.
    pub fn search(&self, query: &str, max_results: usize) -> IhResult<Vec<SearchHit>> {
        if query.is_empty() {
            return Err(IhError::Invalid("requête de recherche vide".to_string()));
        }
        if query.len() > 4_096 {
            return Err(IhError::Invalid(
                "requête de recherche trop longue (max 4096 octets)".to_string(),
            ));
        }
        let max_results = max_results.min(MAX_SEARCH_RESULTS);
        if max_results == 0 {
            return Ok(Vec::new());
        }

        // La requête est toujours un littéral : les métacaractères regex sont
        // échappés, donc aucun motif arbitraire fourni par l'UI n'est exécuté.
        let pattern = regex::escape(query);
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
            .map_err(|error| IhError::Internal(format!("motif de recherche invalide : {error}")))?;

        let walker = WalkDir::new(&self.root)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || !is_ignored(entry));

        let mut hits = Vec::new();
        'files: for entry in walker {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::debug!("entrée ignorée pendant search : {error}");
                    continue;
                }
            };
            // `is_file()` est faux pour un symlink : on ne suit jamais un lien.
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(readable) = readable_text_file(&entry, &self.root) else {
                continue;
            };
            for (line_index, line) in readable.lines().enumerate() {
                let Some(found) = regex.find(line) else {
                    continue;
                };
                hits.push(SearchHit {
                    path: relative_path(&self.root, entry.path()).unwrap_or_default(),
                    line: line_index as u32 + 1,
                    column: found.start() as u32 + 1,
                    preview: preview(line),
                });
                if hits.len() >= max_results {
                    break 'files;
                }
            }
        }
        Ok(hits)
    }

    /// Résout un chemin relatif en vérifiant qu'il reste sous la racine.
    ///
    /// Refuse : chemins vides, absolus, composants `..`, et tout symlink dont
    /// la cible canonicalisée sort du projet.
    pub fn resolve(&self, rel_path: &str) -> IhResult<PathBuf> {
        if rel_path.trim().is_empty() {
            return Err(IhError::Invalid("chemin vide".to_string()));
        }
        let relative = Path::new(rel_path);
        if relative.is_absolute() {
            return Err(IhError::PathEscape(format!(
                "chemin absolu refusé : {rel_path}"
            )));
        }
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(IhError::PathEscape(format!(
                "composant absolu ou `..` refusé : {rel_path}"
            )));
        }

        let candidate = self.root.join(relative);
        let canonical = canonicalize_allow_missing(&candidate)?;
        if !canonical.starts_with(&self.root) {
            return Err(IhError::PathEscape(format!(
                "chemin hors du projet refusé : {rel_path}"
            )));
        }
        Ok(canonical)
    }
}

fn is_ignored(entry: &DirEntry) -> bool {
    IGNORED_DIRS
        .iter()
        .any(|ignored| entry.file_name() == std::ffi::OsStr::new(ignored))
}

/// Canonicalise un chemin dont la fin peut ne pas exister encore (écriture).
/// Le plus proche ancêtre existant est canonicalisé, puis la fin est rajoutée.
fn canonicalize_allow_missing(path: &Path) -> IhResult<PathBuf> {
    let mut current = path.to_path_buf();
    let mut missing: Vec<std::ffi::OsString> = Vec::new();

    loop {
        match sysfs::canonicalize(&current) {
            Ok(mut base) => {
                for component in missing.iter().rev() {
                    base.push(component);
                }
                return Ok(base);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = current.file_name().map(|name| name.to_os_string()) else {
                    return Err(IhError::NotFound(format!(
                        "chemin introuvable : {}",
                        path.display()
                    )));
                };
                missing.push(name);
                if !current.pop() {
                    return Err(IhError::NotFound(format!(
                        "chemin introuvable : {}",
                        path.display()
                    )));
                }
            }
            Err(error) => return Err(IhError::Io(error)),
        }
    }
}

/// Lit un fichier texte éligible à la recherche : régulier, ≤ 2 Mio, sans
/// octet nul. Renvoie `None` si le fichier doit être ignoré.
fn readable_text_file(entry: &DirEntry, root: &Path) -> Option<String> {
    let metadata = entry.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_TEXT_FILE_BYTES as u64 {
        return None;
    }
    // Ceinture et bretelles : la prison s'applique aussi à l'indexation.
    let resolved = sysfs::canonicalize(entry.path()).ok()?;
    if !resolved.starts_with(root) {
        return None;
    }
    let bytes = sysfs::read(entry.path()).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn normalize_rel(path: &Path) -> String {
    let mut parts = Vec::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

fn preview(line: &str) -> String {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut text: String = line.chars().take(PREVIEW_CHARS).collect();
    if line.chars().count() > PREVIEW_CHARS {
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp_project() -> (std::path::PathBuf, ProjectFs) {
        let dir = std::env::temp_dir().join(format!("ih-fs-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let fs = ProjectFs::new(&dir).unwrap();
        (dir, fs)
    }

    #[test]
    fn metadata_hash_taille_et_binarite() {
        let (dir, fs) = temp_project();
        fs.write_text("note.txt", "bonjour").unwrap();
        let meta = fs.metadata("note.txt").unwrap();
        assert_eq!(meta.size, 7);
        assert!(!meta.is_binary);
        assert_eq!(meta.sha256.len(), 64);
        assert_eq!(meta.sha256, sha256_hex(b"bonjour"));

        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        assert!(fs.metadata("bin.dat").unwrap().is_binary);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_checked_refuse_un_ecrasement_si_le_fichier_a_change() {
        let (dir, fs) = temp_project();
        fs.write_text("lib.rs", "fn main() {}\n").unwrap();
        let before = fs.metadata("lib.rs").unwrap();

        // Contenu modifié entre-temps sur le disque.
        std::fs::write(dir.join("lib.rs"), "fn main() { /* autre */ }\n").unwrap();
        let error = fs
            .write_checked(
                "lib.rs",
                "nouveau",
                Some(&before.sha256),
                Some(&dir.join("bak")),
            )
            .unwrap_err();
        assert!(matches!(error, IhError::Conflict(_)));
        assert_eq!(
            std::fs::read_to_string(dir.join("lib.rs")).unwrap(),
            "fn main() { /* autre */ }\n",
            "le fichier ne doit pas être écrasé"
        );

        // Avec l'empreinte courante, l'écriture passe et une sauvegarde est créée.
        let current = fs.metadata("lib.rs").unwrap();
        let outcome = fs
            .write_checked(
                "lib.rs",
                "fini\n",
                Some(&current.sha256),
                Some(&dir.join("bak")),
            )
            .unwrap();
        assert_eq!(outcome.bytes, 5);
        assert!(outcome.backup.is_some());
        assert_eq!(fs.read_text("lib.rs", 1024).unwrap().content, "fini\n");
        let backup = outcome.backup.unwrap();
        assert!(std::path::Path::new(&backup).is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_checked_cree_un_fichier_absent_sans_sauvegarde() {
        let (dir, fs) = temp_project();
        let outcome = fs
            .write_checked("neuf.rs", "x\n", None, Some(&dir.join("bak")))
            .unwrap();
        assert!(outcome.backup.is_none());
        assert!(dir.join("neuf.rs").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }

    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "ih-workspace-{prefix}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("création du dossier temporaire");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn project() -> (TempDir, ProjectFs) {
        let dir = TempDir::new("fs");
        let fs = ProjectFs::new(dir.path()).expect("racine valide");
        (dir, fs)
    }

    #[test]
    fn aller_retour_lecture_ecriture() {
        let (_tmp, project) = project();
        let written = project
            .write_text("src/nested/note.txt", "bonjour")
            .expect("écriture");
        assert_eq!(written, 7);
        assert!(project.root().join("src/nested/note.txt").is_file());

        let content = project.read_text("src/nested/note.txt", 1_024).unwrap();
        assert_eq!(content.content, "bonjour");
        assert_eq!(content.path, "src/nested/note.txt");
        assert!(!content.truncated);
        assert_eq!(content.size, 7);

        // Les accents comptent en octets, pas en caractères.
        assert_eq!(project.write_text("accent.txt", "é").unwrap(), 2);
        assert!(matches!(
            project.read_text("absent.txt", 10),
            Err(IhError::Io(_))
        ));

        // Aucun fichier temporaire ne doit rester après l'écriture atomique.
        let leftovers = std::fs::read_dir(project.root())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".ih-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[cfg(unix)]
    #[test]
    fn ecriture_conserve_le_mode_executable() {
        use std::os::unix::fs::PermissionsExt;

        let (_tmp, project) = project();
        project.write_text("script.sh", "#!/bin/sh\n").unwrap();
        let path = project.root().join("script.sh");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

        project
            .write_text("script.sh", "#!/bin/sh\necho ok\n")
            .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn arborescence_ignoree_et_bornee() {
        let (_tmp, project) = project();
        project.write_text("a.txt", "a").unwrap();
        project.write_text("src/b.rs", "b").unwrap();
        project.write_text(".git/config", "git").unwrap();
        project.write_text("target/debug/x", "x").unwrap();
        project
            .write_text("node_modules/pkg/index.js", "js")
            .unwrap();

        let all = project.list_tree(10, 100).unwrap();
        let paths: Vec<&str> = all.iter().map(|node| node.path.as_str()).collect();
        assert!(paths.contains(&"a.txt"));
        assert!(paths.contains(&"src"));
        assert!(paths.contains(&"src/b.rs"));
        assert!(!paths.iter().any(|path| path.starts_with(".git")));
        assert!(!paths.iter().any(|path| path.starts_with("target")));
        assert!(!paths.iter().any(|path| path.starts_with("node_modules")));

        let shallow = project.list_tree(1, 100).unwrap();
        assert!(shallow.iter().any(|node| node.path == "src"));
        assert!(!shallow.iter().any(|node| node.path == "src/b.rs"));

        let bounded = project.list_tree(10, 2).unwrap();
        assert_eq!(bounded.len(), 2);
        assert!(project.list_tree(0, 100).unwrap().is_empty());
        assert!(project.list_tree(10, 0).unwrap().is_empty());
    }

    #[test]
    fn fichier_volumineux_tronque() {
        let (_tmp, project) = project();
        let big = "x".repeat(100_000);
        project.write_text("gros.txt", &big).unwrap();

        let read = project.read_text("gros.txt", 128).unwrap();
        assert!(read.truncated);
        assert_eq!(read.content.len(), 128);
        assert_eq!(read.size, 100_000);

        // Au-delà de 2 Mio, la lecture est plafonnée même si l'appelant
        // demande davantage.
        let huge = "y".repeat(MAX_TEXT_FILE_BYTES + 4_096);
        project.write_text("enorme.txt", &huge).unwrap();
        let read = project
            .read_text("enorme.txt", MAX_TEXT_FILE_BYTES * 4)
            .unwrap();
        assert!(read.truncated);
        assert_eq!(read.content.len(), MAX_TEXT_FILE_BYTES);
        assert_eq!(read.size, huge.len() as u64);
        assert!(project.search("yyyyyy", 10).unwrap().is_empty());
    }

    #[test]
    fn recherche_trouve_un_motif_borne() {
        let (_tmp, project) = project();
        project
            .write_text("src/main.rs", "fn main() {\n    let motif_cible = 1;\n}\n")
            .unwrap();
        project
            .write_text("notes.txt", "Le motif_cible est ici.\n")
            .unwrap();
        project.write_text("autre.txt", "rien à voir\n").unwrap();

        let hits = project.search("motif_cible", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|hit| hit.line == 2 || hit.line == 1));
        assert!(hits.iter().any(|hit| hit.path == "src/main.rs"));
        assert!(hits.iter().any(|hit| hit.path == "notes.txt"));
        assert!(hits.iter().all(|hit| hit.preview.contains("motif_cible")));

        // Les métacaractères restent littéraux.
        let literal = project.search("fn main()", 10).unwrap();
        assert_eq!(literal.len(), 1);
        assert_eq!(literal[0].path, "src/main.rs");
        assert_eq!(literal[0].column, 1);
        assert!(project.search(".*", 10).unwrap().is_empty());

        let bounded = project.search("motif_cible", 1).unwrap();
        assert_eq!(bounded.len(), 1);
        assert!(project.search("", 10).is_err());
    }

    #[test]
    fn fichiers_binaires_ignores() {
        let (_tmp, project) = project();
        std::fs::write(project.root().join("binaire.dat"), b"\0motif_cible").unwrap();
        project.write_text("texte.txt", "motif_cible\n").unwrap();

        assert!(matches!(
            project.read_text("binaire.dat", 100),
            Err(IhError::Invalid(_))
        ));
        let hits = project.search("motif_cible", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "texte.txt");
    }

    #[cfg(unix)]
    #[test]
    fn prison_de_chemins_et_symlinks() {
        use std::os::unix::fs::symlink;

        let (_tmp, project) = project();
        project.write_text("ok.txt", "ok").unwrap();
        let resolved = project.resolve("ok.txt").unwrap();
        assert!(resolved.starts_with(project.root()));
        assert_eq!(project.resolve("./ok.txt").unwrap(), resolved);

        assert!(matches!(
            project.resolve("../etc/passwd"),
            Err(IhError::PathEscape(_))
        ));
        assert!(matches!(
            project.resolve("/etc/passwd"),
            Err(IhError::PathEscape(_))
        ));
        assert!(matches!(
            project.resolve("src/../ok.txt"),
            Err(IhError::PathEscape(_))
        ));
        assert!(project.resolve("").is_err());

        // Symlink de dossier qui sort du projet.
        symlink("/etc", project.root().join("evade")).unwrap();
        assert!(matches!(
            project.resolve("evade/passwd"),
            Err(IhError::PathEscape(_))
        ));
        assert!(project.read_text("evade/passwd", 10).is_err());
        assert!(project.write_text("evade/pwn", "x").is_err());

        // Symlink de fichier qui sort du projet.
        symlink("/etc/passwd", project.root().join("lien-passwd")).unwrap();
        assert!(matches!(
            project.resolve("lien-passwd"),
            Err(IhError::PathEscape(_))
        ));

        // Symlink interne : autorisé, toujours confiné.
        symlink(
            project.root().join("ok.txt"),
            project.root().join("lien-ok"),
        )
        .unwrap();
        let inside = project.resolve("lien-ok").unwrap();
        assert!(inside.starts_with(project.root()));
        assert_eq!(project.read_text("lien-ok", 10).unwrap().content, "ok");
    }
}
