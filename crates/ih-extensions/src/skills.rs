//! Skills : manifeste, installation vérifiée par SHA-256 et désinstallation.
//!
//! Un skill est du contenu d'instruction, jamais un exécutable : l'installation
//! ne fait que copier des fichiers, sans suivre aucun lien symbolique et sans
//! exécuter quoi que ce soit. L'empreinte SHA-256 est calculée de façon
//! déterministe pour permettre de vérifier un paquet installé plus tard.
//!
//! ## Format de l'empreinte
//!
//! Les fichiers réguliers du paquet sont triés par chemin relatif (octets
//! croissants, séparateur `/`). Pour chacun, on injecte dans SHA-256 :
//!
//! ```text
//! longueur_du_chemin (u64 big-endian) || chemin UTF-8
//! longueur_du_contenu (u64 big-endian) || contenu
//! ```
//!
//! Les dossiers vides et les permissions de fichiers ne sont pas couverts ;
//! les liens symboliques sont refusés avant tout calcul.

use ih_core::{IhError, IhResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

/// Manifeste d'un skill, tel que fourni dans `skill.json` (ou `manifest.json`).
///
/// `permissions` accepte aussi le nom historique `capabilities` du schéma du
/// dépôt ; les valeurs `project-read` et `project-write` y sont normalisées en
/// `fs.read` et `fs.write`. `mcp-tools` n'a pas d'équivalent et est refusé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    #[serde(default)]
    pub license: Option<String>,
    /// Fichier markdown principal, relatif au dossier du skill (ex. `SKILL.md`).
    pub entry: String,
    #[serde(default, alias = "capabilities")]
    pub permissions: Vec<String>,
}

/// Résultat d'une installation réussie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub files: usize,
}

/// Skill présent dans une racine d'installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkill {
    pub manifest: SkillManifest,
    pub dir: PathBuf,
    pub sha256: String,
}

/// Longueur maximale acceptée pour un identifiant ou une version.
const MAX_ID_LEN: usize = 64;
const MAX_VERSION_LEN: usize = 64;
const MAX_PERMISSIONS: usize = 32;
const MAX_ENTRY_LEN: usize = 512;

/// Analyse et valide un manifeste de skill.
pub fn parse_manifest(json: &str) -> IhResult<SkillManifest> {
    let mut manifest: SkillManifest = serde_json::from_str(json)
        .map_err(|error| invalid_manifest(format!("JSON invalide ou champ manquant : {error}")))?;

    validate_id(&manifest.id)?;
    validate_semver(&manifest.version)?;
    validate_entry(&manifest.entry)?;

    if manifest.name.trim().is_empty() {
        return Err(invalid_manifest("le champ « name » est vide"));
    }
    if manifest.author.trim().is_empty() {
        return Err(invalid_manifest("le champ « author » est vide"));
    }
    if manifest.permissions.len() > MAX_PERMISSIONS {
        return Err(invalid_manifest(format!(
            "trop de permissions ({} > {MAX_PERMISSIONS})",
            manifest.permissions.len()
        )));
    }

    let mut normalized: Vec<String> = Vec::with_capacity(manifest.permissions.len());
    for permission in &manifest.permissions {
        let canonical = normalize_permission(permission).ok_or_else(|| {
            invalid_manifest(format!(
                "permission inconnue « {permission} » (valeurs autorisées : fs.read, fs.write, shell, network)"
            ))
        })?;
        if !normalized.iter().any(|known| known == canonical) {
            normalized.push(canonical.to_string());
        }
    }
    normalized.sort();
    manifest.permissions = normalized;

    Ok(manifest)
}

/// Charge le manifeste d'un dossier de skill (`skill.json`, sinon `manifest.json`).
pub fn load_manifest(dir: &Path) -> IhResult<SkillManifest> {
    for name in ["skill.json", "manifest.json"] {
        let path = dir.join(name);
        match fs::read_to_string(&path) {
            Ok(text) => {
                return parse_manifest(&text).map_err(|error| {
                    IhError::Extension(format!(
                        "manifeste invalide ({}) : {}",
                        path.display(),
                        extension_message(error)
                    ))
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
                return Err(IhError::Extension(format!(
                    "manifeste non UTF-8 : {}",
                    path.display()
                )));
            }
            Err(error) => return Err(IhError::Io(error)),
        }
    }
    Err(IhError::Extension(format!(
        "aucun manifeste de skill (skill.json ou manifest.json) dans {}",
        dir.display()
    )))
}

/// Installe un skill dans `<skills_root>/<id>-<version>/`.
///
/// Refuse les liens symboliques, une source identique à la destination, une
/// destination déjà existante et tout chemin qui sortirait de la racine.
pub fn install_skill(source_dir: &Path, skills_root: &Path) -> IhResult<InstallReport> {
    let metadata = fs::symlink_metadata(source_dir).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            IhError::NotFound(format!(
                "dossier source introuvable : {}",
                source_dir.display()
            ))
        } else {
            IhError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(invalid_manifest(format!(
            "le dossier source est un lien symbolique, refusé : {}",
            source_dir.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(invalid_manifest(format!(
            "la source n'est pas un dossier : {}",
            source_dir.display()
        )));
    }

    let source = fs::canonicalize(source_dir).map_err(IhError::Io)?;
    let manifest = load_manifest(&source)?;

    fs::create_dir_all(skills_root).map_err(IhError::Io)?;
    let root = fs::canonicalize(skills_root).map_err(IhError::Io)?;
    if !root.is_dir() {
        return Err(IhError::Extension(format!(
            "la racine des skills n'est pas un dossier : {}",
            root.display()
        )));
    }

    let name = format!("{}-{}", manifest.id, manifest.version);
    let destination = root.join(&name);
    if destination == source {
        return Err(IhError::Extension(
            "la source et la destination de l'installation sont identiques".into(),
        ));
    }
    if destination.starts_with(&source) {
        return Err(IhError::Extension(format!(
            "la destination {} est à l'intérieur de la source {}",
            destination.display(),
            source.display()
        )));
    }
    if source.starts_with(&destination) {
        return Err(IhError::Extension(format!(
            "la source {} est à l'intérieur de la destination {}",
            source.display(),
            destination.display()
        )));
    }
    if destination.parent() != Some(root.as_path()) {
        return Err(IhError::PathEscape(format!(
            "destination hors de la racine des skills : {}",
            destination.display()
        )));
    }
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(IhError::Conflict(format!(
                "skill déjà installé : {}",
                destination.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(IhError::Io(error)),
    }

    let entry = source.join(&manifest.entry);
    match fs::symlink_metadata(&entry) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(invalid_manifest(format!(
                "l'entrée du skill est un lien symbolique, refusé : {}",
                entry.display()
            )));
        }
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            return Err(invalid_manifest(format!(
                "l'entrée du skill n'est pas un fichier : {}",
                entry.display()
            )));
        }
        Err(_) => {
            return Err(invalid_manifest(format!(
                "l'entrée « {} » est introuvable dans la source",
                manifest.entry
            )));
        }
    }

    let files = collect_files(&source)?;
    let sha256 = hash_files(&files)?;

    if let Err(error) = copy_files(&files, &destination) {
        remove_if_present(&destination);
        return Err(error);
    }

    match directory_sha256(&destination) {
        Ok(copied) if copied == sha256 => {}
        Ok(copied) => {
            remove_if_present(&destination);
            return Err(IhError::Internal(format!(
                "empreinte incohérente après copie (attendu {sha256}, obtenu {copied})"
            )));
        }
        Err(error) => {
            remove_if_present(&destination);
            return Err(error);
        }
    }

    tracing::debug!(
        skill = %manifest.id,
        version = %manifest.version,
        files = files.len(),
        installed_at = %chrono::Utc::now().to_rfc3339(),
        "skill installé"
    );

    Ok(InstallReport {
        id: manifest.id,
        version: manifest.version,
        sha256,
        files: files.len(),
    })
}

/// Vérifie qu'un dossier installé correspond à l'empreinte attendue.
pub fn verify_installed(skill_dir: &Path, expected_sha256: &str) -> IhResult<bool> {
    validate_sha256(expected_sha256)?;
    let metadata = fs::symlink_metadata(skill_dir).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            IhError::NotFound(format!(
                "skill installé introuvable : {}",
                skill_dir.display()
            ))
        } else {
            IhError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(IhError::Extension(format!(
            "refus de vérifier un lien symbolique : {}",
            skill_dir.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(IhError::Extension(format!(
            "le skill installé n'est pas un dossier : {}",
            skill_dir.display()
        )));
    }
    let actual = directory_sha256(skill_dir)?;
    Ok(actual.eq_ignore_ascii_case(expected_sha256))
}

/// Liste les skills installés sous `skills_root` (racine absente = liste vide).
pub fn list_installed(skills_root: &Path) -> IhResult<Vec<InstalledSkill>> {
    let root = match fs::canonicalize(skills_root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(IhError::Io(error)),
    };
    if !root.is_dir() {
        return Err(IhError::Extension(format!(
            "la racine des skills n'est pas un dossier : {}",
            root.display()
        )));
    }

    let mut installed = Vec::new();
    for entry in fs::read_dir(&root).map_err(IhError::Io)? {
        let entry = entry.map_err(IhError::Io)?;
        let file_type = entry.file_type().map_err(IhError::Io)?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let dir = entry.path();
        let manifest = load_manifest(&dir).map_err(|error| {
            IhError::Extension(format!(
                "skill installé invalide dans {} : {}",
                dir.display(),
                extension_message(error)
            ))
        })?;
        let sha256 = directory_sha256(&dir)?;
        installed.push(InstalledSkill {
            manifest,
            dir,
            sha256,
        });
    }
    installed.sort_by(|left, right| {
        (&left.manifest.id, &left.manifest.version)
            .cmp(&(&right.manifest.id, &right.manifest.version))
    });
    Ok(installed)
}

/// Supprime `<skills_root>/<id>-<version>/`, et rien d'autre.
pub fn remove_skill(skills_root: &Path, id: &str, version: &str) -> IhResult<()> {
    validate_id(id)?;
    validate_semver(version)?;

    let root = fs::canonicalize(skills_root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            IhError::NotFound(format!(
                "racine des skills introuvable : {}",
                skills_root.display()
            ))
        } else {
            IhError::Io(error)
        }
    })?;
    if !root.is_dir() {
        return Err(IhError::Extension(format!(
            "la racine des skills n'est pas un dossier : {}",
            root.display()
        )));
    }

    let candidate = root.join(format!("{id}-{version}"));
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            IhError::NotFound(format!("skill installé introuvable : {id}-{version}"))
        } else {
            IhError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(IhError::PathEscape(format!(
            "refus de supprimer un lien symbolique : {}",
            candidate.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(IhError::Extension(format!(
            "le skill installé n'est pas un dossier : {}",
            candidate.display()
        )));
    }

    let destination = fs::canonicalize(&candidate).map_err(IhError::Io)?;
    if destination == root || !destination.starts_with(&root) {
        return Err(IhError::PathEscape(format!(
            "suppression hors de la racine des skills refusée : {}",
            destination.display()
        )));
    }

    fs::remove_dir_all(&destination).map_err(IhError::Io)?;
    tracing::debug!(skill = %id, version = %version, "skill désinstallé");
    Ok(())
}

/// Calcule l'empreinte SHA-256 canonique d'un dossier de skill.
pub fn directory_sha256(dir: &Path) -> IhResult<String> {
    let files = collect_files(dir)?;
    hash_files(&files)
}

fn invalid_manifest(message: impl Into<String>) -> IhError {
    let message = message.into();
    IhError::Extension(format!("manifeste de skill invalide : {message}"))
}

fn extension_message(error: IhError) -> String {
    match error {
        IhError::Extension(message) => message,
        other => other.to_string(),
    }
}

fn validate_id(id: &str) -> IhResult<()> {
    if id.is_empty() {
        return Err(invalid_manifest("l'identifiant « id » est vide"));
    }
    if id.len() > MAX_ID_LEN {
        return Err(invalid_manifest(format!(
            "l'identifiant « id » dépasse {MAX_ID_LEN} caractères"
        )));
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(invalid_manifest(format!(
            "l'identifiant « id » contient un séparateur ou « .. » : {id}"
        )));
    }
    // kebab-case strict : segments alphanumériques minuscules séparés par un
    // seul tiret, sans tiret en tête ni en fin.
    let kebab = !id.starts_with('-')
        && !id.ends_with('-')
        && id.split('-').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !kebab {
        return Err(invalid_manifest(format!(
            "l'identifiant « id » doit être en kebab-case (a-z, 0-9, tirets) : {id}"
        )));
    }
    Ok(())
}

fn validate_semver(version: &str) -> IhResult<()> {
    let invalid =
        |reason: &str| invalid_manifest(format!("version « {version} » invalide : {reason}"));

    if version.is_empty() {
        return Err(invalid("version vide"));
    }
    if version.len() > MAX_VERSION_LEN {
        return Err(invalid("version trop longue"));
    }
    if version.trim() != version {
        return Err(invalid("espaces en trop"));
    }

    let core_and_prerelease = match version.split_once('+') {
        Some((head, build)) => {
            if build.is_empty() || build.contains('+') {
                return Err(invalid("métadonnées de build vides ou multiples"));
            }
            for identifier in build.split('.') {
                if identifier.is_empty()
                    || !identifier
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(invalid("métadonnées de build invalides"));
                }
            }
            head
        }
        None => version,
    };

    let core = match core_and_prerelease.split_once('-') {
        Some((core, prerelease)) => {
            if prerelease.is_empty() {
                return Err(invalid("pré-version vide"));
            }
            for identifier in prerelease.split('.') {
                if identifier.is_empty()
                    || !identifier
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return Err(invalid("pré-version invalide"));
                }
                if identifier.len() > 1
                    && identifier.starts_with('0')
                    && identifier.bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(invalid("pré-version numérique avec zéro initial"));
                }
            }
            core
        }
        None => core_and_prerelease,
    };

    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(invalid("format attendu : majeur.mineur.correctif"));
    }
    for (label, part) in ["majeure", "mineure", "correctif"].iter().zip(&parts) {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid(&format!("composante {label} non numérique")));
        }
        if part.len() > 1 && part.starts_with('0') {
            return Err(invalid(&format!("composante {label} avec zéro initial")));
        }
    }
    Ok(())
}

fn validate_entry(entry: &str) -> IhResult<()> {
    if entry.is_empty() {
        return Err(invalid_manifest("le champ « entry » est vide"));
    }
    if entry.len() > MAX_ENTRY_LEN {
        return Err(invalid_manifest("le champ « entry » est trop long"));
    }
    if entry.contains('\\') {
        return Err(invalid_manifest(format!(
            "le champ « entry » ne doit pas contenir d'antislash : {entry}"
        )));
    }
    let path = Path::new(entry);
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {
                return Err(invalid_manifest(format!(
                    "le champ « entry » ne doit pas contenir « . » : {entry}"
                )));
            }
            _ => {
                return Err(invalid_manifest(format!(
                    "le champ « entry » doit être relatif et sans « .. » : {entry}"
                )));
            }
        }
    }
    if entry.starts_with('/') || entry.split('/').any(str::is_empty) {
        return Err(invalid_manifest(format!(
            "le champ « entry » est un chemin invalide : {entry}"
        )));
    }
    let markdown = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    if !markdown {
        return Err(invalid_manifest(format!(
            "le champ « entry » doit désigner un fichier markdown (.md) : {entry}"
        )));
    }
    Ok(())
}

fn normalize_permission(permission: &str) -> Option<&'static str> {
    match permission.trim() {
        "fs.read" | "project-read" => Some("fs.read"),
        "fs.write" | "project-write" => Some("fs.write"),
        "shell" => Some("shell"),
        "network" => Some("network"),
        _ => None,
    }
}

fn validate_sha256(value: &str) -> IhResult<()> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(IhError::Invalid(format!(
            "empreinte SHA-256 invalide (64 caractères hexadécimaux attendus) : {value}"
        )))
    }
}

struct SourceFile {
    relative: String,
    absolute: PathBuf,
    len: u64,
}

fn collect_files(root: &Path) -> IhResult<Vec<SourceFile>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = entry.map_err(|error| {
            IhError::Extension(format!("parcours du skill impossible : {error}"))
        })?;
        if entry.depth() == 0 {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(IhError::Extension(format!(
                "lien symbolique refusé dans le skill : {}",
                entry.path().display()
            )));
        }
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Err(IhError::Extension(format!(
                "type de fichier non pris en charge dans le skill : {}",
                entry.path().display()
            )));
        }

        let relative = entry.path().strip_prefix(root).map_err(|_| {
            IhError::Internal(format!(
                "chemin hors du dossier du skill : {}",
                entry.path().display()
            ))
        })?;
        let relative = relative.to_str().ok_or_else(|| {
            IhError::Extension(format!(
                "nom de fichier non UTF-8 refusé : {}",
                entry.path().display()
            ))
        })?;
        // Le hash doit être identique sur tous les systèmes : séparateur `/`.
        let relative = relative.replace(std::path::MAIN_SEPARATOR, "/");
        if relative.contains('\\') {
            return Err(IhError::Extension(format!(
                "nom de fichier contenant un antislash refusé : {relative}"
            )));
        }

        let metadata = fs::symlink_metadata(entry.path()).map_err(IhError::Io)?;
        if !metadata.is_file() {
            return Err(IhError::Extension(format!(
                "type de fichier non pris en charge dans le skill : {}",
                entry.path().display()
            )));
        }
        files.push(SourceFile {
            relative: relative.to_string(),
            absolute: entry.path().to_path_buf(),
            len: metadata.len(),
        });
    }

    if files.is_empty() {
        return Err(IhError::Extension(format!(
            "le dossier du skill ne contient aucun fichier : {}",
            root.display()
        )));
    }

    files.sort_by(|left, right| left.relative.as_bytes().cmp(right.relative.as_bytes()));
    Ok(files)
}

fn hash_files(files: &[SourceFile]) -> IhResult<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];

    for file in files {
        let path = file.relative.as_bytes();
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path);
        hasher.update(file.len.to_be_bytes());

        let mut handle = fs::File::open(&file.absolute).map_err(IhError::Io)?;
        let mut read_total: u64 = 0;
        loop {
            let read = handle.read(&mut buffer).map_err(IhError::Io)?;
            if read == 0 {
                break;
            }
            read_total += read as u64;
            hasher.update(&buffer[..read]);
        }
        if read_total != file.len {
            return Err(IhError::Extension(format!(
                "le fichier {} a changé de taille pendant l'installation",
                file.relative
            )));
        }
    }

    Ok(hex::encode(hasher.finalize()))
}

fn copy_files(files: &[SourceFile], destination: &Path) -> IhResult<()> {
    fs::create_dir_all(destination).map_err(IhError::Io)?;
    for file in files {
        let target = destination.join(&file.relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(IhError::Io)?;
        }
        fs::copy(&file.absolute, &target).map_err(IhError::Io)?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) {
    if fs::symlink_metadata(path).is_ok() {
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{write_file, TempDir};
    use serde_json::json;

    fn manifest_value() -> serde_json::Value {
        json!({
            "id": "mon-skill",
            "name": "Mon skill",
            "version": "1.2.3",
            "description": "Un skill de test",
            "author": "IH",
            "license": "MIT",
            "entry": "SKILL.md",
            "permissions": ["shell", "fs.read"]
        })
    }

    fn valid_json() -> String {
        serde_json::to_string(&manifest_value()).unwrap()
    }

    fn make_skill(source: &Path) {
        write_file(&source.join("skill.json"), valid_json());
        write_file(&source.join("SKILL.md"), "# Mon skill\n\nInstructions.\n");
        write_file(&source.join("assets/data.txt"), "contenu\n");
    }

    #[test]
    fn parse_manifest_valide() {
        let manifest = parse_manifest(&valid_json()).unwrap();
        assert_eq!(manifest.id, "mon-skill");
        assert_eq!(manifest.version, "1.2.3");
        assert_eq!(manifest.entry, "SKILL.md");
        assert_eq!(manifest.license.as_deref(), Some("MIT"));
        // Les permissions sont normalisées et triées.
        assert_eq!(manifest.permissions, vec!["fs.read", "shell"]);
    }

    #[test]
    fn parse_manifest_accepte_le_schema_historique_capabilities() {
        let mut value = manifest_value();
        value["capabilities"] = json!(["project-read", "project-write"]);
        value.as_object_mut().unwrap().remove("permissions");
        let manifest = parse_manifest(&value.to_string()).unwrap();
        assert_eq!(manifest.permissions, vec!["fs.read", "fs.write"]);
    }

    #[test]
    fn parse_manifest_rejette_un_id_invalide() {
        for id in ["../x", "a/b", "A-majuscule", "", "-debut", "fin-", "a..b"] {
            let mut value = manifest_value();
            value["id"] = json!(id);
            let error = parse_manifest(&value.to_string()).unwrap_err();
            assert!(
                matches!(&error, IhError::Extension(_)),
                "id {id:?} devrait être refusé : {error}"
            );
            assert!(
                error.to_string().contains("id"),
                "message imprécis pour {id:?} : {error}"
            );
        }
    }

    #[test]
    fn parse_manifest_rejette_une_version_non_semver() {
        for version in [
            "",
            "1.2",
            "v1.2.3",
            "01.2.3",
            "1.02.3",
            "1.2.3-",
            "1.2.3+",
            "1.2.3+build+encore",
            "1.2.3-alpha..1",
            "1.2.3-01",
            "1.2.3.4",
        ] {
            let mut value = manifest_value();
            value["version"] = json!(version);
            let error = parse_manifest(&value.to_string()).unwrap_err();
            assert!(
                error.to_string().contains("version"),
                "message imprécis pour {version:?} : {error}"
            );
        }
        // Formes valides.
        for version in ["0.0.1", "10.20.30", "1.2.3-alpha.1", "1.2.3+build.5"] {
            let mut value = manifest_value();
            value["version"] = json!(version);
            parse_manifest(&value.to_string())
                .unwrap_or_else(|error| panic!("version {version:?} refusée : {error}"));
        }
    }

    #[test]
    fn parse_manifest_rejette_une_permission_inconnue() {
        for permission in ["root", "fs.execute", "mcp-tools"] {
            let mut value = manifest_value();
            value["permissions"] = json!([permission]);
            let error = parse_manifest(&value.to_string()).unwrap_err();
            assert!(matches!(&error, IhError::Extension(_)));
            assert!(
                error.to_string().contains("permission inconnue"),
                "message imprécis : {error}"
            );
        }
    }

    #[test]
    fn parse_manifest_rejette_un_entry_avec_traversee() {
        for entry in [
            "../SKILL.md",
            "docs/../../SKILL.md",
            "/etc/passwd.md",
            "./SKILL.md",
            "notes.txt",
            "dossier//SKILL.md",
            "dossier\\SKILL.md",
        ] {
            let mut value = manifest_value();
            value["entry"] = json!(entry);
            let error = parse_manifest(&value.to_string()).unwrap_err();
            assert!(
                matches!(&error, IhError::Extension(_)),
                "entry {entry:?} devrait être refusé : {error}"
            );
        }
    }

    #[test]
    fn load_manifest_accepte_manifest_json_et_refuse_l_absence() {
        let temp = TempDir::new("load-manifest");
        let dir = temp.join("skill");
        fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("manifest.json"), valid_json());
        assert_eq!(load_manifest(&dir).unwrap().id, "mon-skill");

        let empty = temp.join("vide");
        fs::create_dir_all(&empty).unwrap();
        let error = load_manifest(&empty).unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(error.to_string().contains("aucun manifeste"));
    }

    #[test]
    fn cycle_installation_liste_verification_desinstallation() {
        let temp = TempDir::new("install-cycle");
        let source = temp.join("source");
        let root = temp.join("skills");
        make_skill(&source);

        let report = install_skill(&source, &root).unwrap();
        assert_eq!(report.id, "mon-skill");
        assert_eq!(report.version, "1.2.3");
        assert_eq!(report.files, 3);
        assert_eq!(report.sha256.len(), 64);
        assert!(report.sha256.bytes().all(|b| b.is_ascii_hexdigit()));

        let installed = root.join("mon-skill-1.2.3");
        assert!(installed.join("SKILL.md").is_file());
        assert!(installed.join("assets/data.txt").is_file());

        let listed = list_installed(&root).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].manifest.id, "mon-skill");
        assert_eq!(listed[0].dir, installed);
        assert_eq!(listed[0].sha256, report.sha256);
        assert_eq!(directory_sha256(&installed).unwrap(), report.sha256);

        assert!(verify_installed(&installed, &report.sha256).unwrap());
        assert!(verify_installed(&installed, &report.sha256.to_ascii_uppercase()).unwrap());

        write_file(&installed.join("SKILL.md"), "# Modifié\n");
        assert!(!verify_installed(&installed, &report.sha256).unwrap());

        remove_skill(&root, "mon-skill", "1.2.3").unwrap();
        assert!(!installed.exists());
        assert!(list_installed(&root).unwrap().is_empty());
        assert!(matches!(
            remove_skill(&root, "mon-skill", "1.2.3").unwrap_err(),
            IhError::NotFound(_)
        ));
    }

    #[test]
    fn empreinte_deterministe_quel_que_soit_l_ordre_de_creation() {
        let temp = TempDir::new("install-determinisme");
        let first = temp.join("premier");
        let second = temp.join("second");
        for dir in [&first, &second] {
            fs::create_dir_all(dir).unwrap();
        }
        write_file(&first.join("skill.json"), valid_json());
        write_file(&first.join("SKILL.md"), "# Mon skill\n");
        write_file(&first.join("assets/data.txt"), "contenu\n");

        // Même paquet, fichiers créés dans l'ordre inverse.
        write_file(&second.join("assets/data.txt"), "contenu\n");
        write_file(&second.join("SKILL.md"), "# Mon skill\n");
        write_file(&second.join("skill.json"), valid_json());

        let report_first = install_skill(&first, &temp.join("racine-a")).unwrap();
        let report_second = install_skill(&second, &temp.join("racine-b")).unwrap();
        assert_eq!(report_first.sha256, report_second.sha256);
    }

    #[test]
    fn desinstallation_ne_supprime_que_la_version_demandee() {
        let temp = TempDir::new("remove-version");
        let root = temp.join("skills");
        let first = temp.join("source-v1");
        make_skill(&first);

        let second = temp.join("source-v2");
        fs::create_dir_all(&second).unwrap();
        let mut value = manifest_value();
        value["version"] = json!("2.0.0");
        value["entry"] = json!("SKILL.md");
        write_file(
            &second.join("skill.json"),
            serde_json::to_string(&value).unwrap(),
        );
        write_file(&second.join("SKILL.md"), "# v2\n");

        install_skill(&first, &root).unwrap();
        install_skill(&second, &root).unwrap();
        assert_eq!(list_installed(&root).unwrap().len(), 2);

        remove_skill(&root, "mon-skill", "1.2.3").unwrap();
        let remaining = list_installed(&root).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].manifest.version, "2.0.0");
        assert!(root.join("mon-skill-2.0.0").is_dir());
    }

    #[test]
    fn installation_refuse_une_destination_existante() {
        let temp = TempDir::new("install-doublon");
        let source = temp.join("source");
        let root = temp.join("skills");
        make_skill(&source);
        install_skill(&source, &root).unwrap();
        let error = install_skill(&source, &root).unwrap_err();
        assert!(matches!(&error, IhError::Conflict(_)));
        assert!(error.to_string().contains("déjà installé"));
    }

    #[test]
    fn installation_refuse_une_source_inexistante() {
        let temp = TempDir::new("install-absente");
        let root = temp.join("skills");
        let error = install_skill(&temp.join("absente"), &root).unwrap_err();
        assert!(matches!(&error, IhError::NotFound(_)));
        assert!(!root.exists(), "la racine ne doit pas être créée");
    }

    #[test]
    fn installation_refuse_la_source_comme_destination() {
        let temp = TempDir::new("install-identique");
        let root = temp.join("skills");
        let source = root.join("mon-skill-1.2.3");
        fs::create_dir_all(&source).unwrap();
        make_skill(&source);
        let error = install_skill(&source, &root).unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(
            error.to_string().contains("identiques"),
            "message imprécis : {error}"
        );
    }

    #[test]
    fn installation_refuse_un_manifeste_invalide() {
        let temp = TempDir::new("install-invalide");
        let source = temp.join("source");
        fs::create_dir_all(&source).unwrap();
        write_file(&source.join("SKILL.md"), "# x\n");
        let mut value = manifest_value();
        value["id"] = json!("../evasion");
        write_file(&source.join("skill.json"), value.to_string());

        let error = install_skill(&source, &temp.join("skills")).unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(error.to_string().contains("id"));
    }

    #[test]
    fn installation_refuse_une_entree_absente() {
        let temp = TempDir::new("install-entry");
        let source = temp.join("source");
        fs::create_dir_all(&source).unwrap();
        write_file(&source.join("skill.json"), valid_json());
        let error = install_skill(&source, &temp.join("skills")).unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(error.to_string().contains("introuvable"));
    }

    #[cfg(unix)]
    #[test]
    fn installation_refuse_les_liens_symboliques() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("install-symlink");
        let outside = temp.join("hors-skill.txt");
        write_file(&outside, "secret\n");

        let source = temp.join("source");
        make_skill(&source);
        symlink(&outside, source.join("lien.txt")).unwrap();
        let error = install_skill(&source, &temp.join("skills")).unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(
            error.to_string().contains("lien symbolique"),
            "message imprécis : {error}"
        );

        // Un dossier source qui est lui-même un lien symbolique est refusé.
        let link = temp.join("source-lien");
        symlink(&source, &link).unwrap();
        let error = install_skill(&link, &temp.join("skills2")).unwrap_err();
        assert!(matches!(&error, IhError::Extension(_)));
        assert!(error.to_string().contains("lien symbolique"));
    }

    #[cfg(unix)]
    #[test]
    fn desinstallation_ne_suit_pas_un_lien_symbolique_hors_racine() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new("remove-symlink");
        let root = temp.join("skills");
        fs::create_dir_all(&root).unwrap();
        let outside = temp.join("ailleurs");
        write_file(&outside.join("précieux.txt"), "à garder\n");
        symlink(&outside, root.join("evil-1.0.0")).unwrap();

        let error = remove_skill(&root, "evil", "1.0.0").unwrap_err();
        assert!(matches!(
            &error,
            IhError::Extension(_) | IhError::PathEscape(_)
        ));
        assert!(outside.join("précieux.txt").is_file());
        assert!(root.join("evil-1.0.0").exists());
    }

    #[test]
    fn desinstallation_refuse_un_id_avec_traversee() {
        let temp = TempDir::new("remove-traversee");
        let root = temp.join("skills");
        fs::create_dir_all(&root).unwrap();
        let error = remove_skill(&root, "../evil", "1.0.0").unwrap_err();
        assert!(
            error.to_string().contains("id"),
            "message imprécis : {error}"
        );
    }

    #[test]
    fn liste_vide_quand_la_racine_est_absente() {
        let temp = TempDir::new("list-absente");
        assert!(list_installed(&temp.join("skills")).unwrap().is_empty());
    }

    #[test]
    fn verification_refuse_une_empreinte_mal_formee() {
        let temp = TempDir::new("verify-format");
        let dir = temp.join("skill");
        fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("SKILL.md"), "# x\n");
        let error = verify_installed(&dir, "pas-hex").unwrap_err();
        assert!(matches!(&error, IhError::Invalid(_)));
    }
}
