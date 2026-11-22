//! Préparation et application des mises à jour.
//!
//! L'application d'un paquet `deb`/`pacman` n'est **jamais** exécutée ici :
//! [`plan_install`] renvoie la commande exacte que l'interface doit présenter à
//! l'utilisateur et faire confirmer. Seule une AppImage (fichier possédé par
//! l'utilisateur) est remplacée directement, après copie de sauvegarde.

use crate::client::UpdateClient;
use crate::crypto::verify_digest;
use crate::manifest::{target_key, Artifact, ArtifactKind, UpdateManifest};
use chrono::Utc;
use ih_core::{IhError, IhResult};
use rand::Rng;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Mise à jour téléchargée, vérifiée et prête à être installée.
///
/// La signature du manifeste doit avoir été vérifiée en amont (par
/// [`UpdateClient::fetch_verified`]). L'empreinte de l'artefact, elle, est
/// revérifiée par [`StagedUpdate::prepare`].
#[derive(Debug, Clone)]
pub struct StagedUpdate {
    pub manifest: UpdateManifest,
    pub artifact: Artifact,
    pub path: PathBuf,
}

impl StagedUpdate {
    /// Télécharge l'artefact du type demandé dans `staging_dir` puis vérifie
    /// son empreinte SHA-256.
    ///
    /// Le nom du fichier local est **constant** et choisi par ce crate : aucun
    /// chemin issu du manifeste n'est utilisé pour écrire. Un digest invalide
    /// (ou une taille incorrecte) supprime le fichier téléchargé et renvoie une
    /// erreur.
    pub async fn prepare(
        client: &UpdateClient,
        manifest: &UpdateManifest,
        kind: ArtifactKind,
        staging_dir: &Path,
    ) -> IhResult<Self> {
        let key = target_key(kind);
        let artifact =
            manifest.platforms.get(key).cloned().ok_or_else(|| {
                IhError::Update(format!("aucun artefact {key} dans le manifeste"))
            })?;
        if artifact.kind != kind {
            return Err(IhError::Update(format!(
                "type d'artefact incohérent pour {key} : {:?}",
                artifact.kind
            )));
        }

        tokio::fs::create_dir_all(staging_dir).await?;
        let metadata = tokio::fs::metadata(staging_dir).await?;
        if !metadata.is_dir() {
            return Err(IhError::Update(format!(
                "le dossier de staging n'est pas un dossier : {}",
                staging_dir.display()
            )));
        }

        // Nom constant, jamais dérivé du manifeste : pas d'écriture hors du
        // dossier de staging, même si l'URL ou le manifeste est hostile.
        let path = staging_dir.join(staged_file_name(kind));

        client.download(&artifact, &path).await?;

        let verified_path = path.clone();
        let expected = artifact.sha256.clone();
        let verification =
            tokio::task::spawn_blocking(move || verify_digest(&verified_path, &expected))
                .await
                .map_err(|err| {
                    IhError::Internal(format!("vérification SHA-256 interrompue : {err}"))
                })?;

        if let Err(err) = verification {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(err);
        }

        tracing::info!(
            version = %manifest.version,
            path = %path.display(),
            "mise à jour préparée et vérifiée"
        );
        Ok(Self {
            manifest: manifest.clone(),
            artifact,
            path,
        })
    }
}

/// Plan d'installation calculé mais non exécuté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPlan {
    /// Remplacement en place d'une AppImage possédée par l'utilisateur.
    ReplaceAppImage { target: PathBuf, staged: PathBuf },
    /// Commande de gestionnaire de paquets à faire confirmer par l'utilisateur.
    RunPackageManager { command: String, args: Vec<String> },
}

/// Calcule le plan d'installation sans rien exécuter.
///
/// * AppImage → [`InstallPlan::ReplaceAppImage`], remplacement en place.
/// * deb → `pkexec apt-get install -y <fichier>`.
/// * pacman → `pkexec pacman -U --noconfirm <fichier>`.
///
/// Les commandes `deb`/`pacman` sont retournées telles quelles : c'est à l'UI
/// d'afficher la commande exacte et d'obtenir une confirmation explicite.
pub fn plan_install(update: &StagedUpdate, current_exe: &Path) -> IhResult<InstallPlan> {
    if current_exe.as_os_str().is_empty() {
        return Err(IhError::Invalid(
            "chemin de l'exécutable courant vide".into(),
        ));
    }

    match update.artifact.kind {
        ArtifactKind::AppImage => Ok(InstallPlan::ReplaceAppImage {
            target: current_exe.to_path_buf(),
            staged: update.path.clone(),
        }),
        ArtifactKind::Deb => {
            let staged = staged_arg(&update.path)?;
            Ok(InstallPlan::RunPackageManager {
                command: "pkexec".into(),
                args: vec!["apt-get".into(), "install".into(), "-y".into(), staged],
            })
        }
        ArtifactKind::Pacman => {
            let staged = staged_arg(&update.path)?;
            Ok(InstallPlan::RunPackageManager {
                command: "pkexec".into(),
                args: vec!["pacman".into(), "-U".into(), "--noconfirm".into(), staged],
            })
        }
    }
}

/// Remplace l'AppImage cible par l'artefact préparé.
///
/// Garanties :
/// 1. l'ancien binaire est copié dans `backup_dir` sous un nom horodaté et sa
///    taille est contrôlée avant tout remplacement ;
/// 2. le nouveau binaire est écrit dans un fichier temporaire du **même
///    dossier** que la cible, avec les permissions de l'ancien fichier plus le
///    bit exécutable ;
/// 3. le remplacement se fait par `rename` atomique.
///
/// Retourne le chemin de la sauvegarde. En cas d'échec, l'ancien binaire reste
/// intact.
pub fn apply_appimage(plan: &InstallPlan, backup_dir: &Path) -> IhResult<PathBuf> {
    let (target, staged) = match plan {
        InstallPlan::ReplaceAppImage { target, staged } => (target, staged),
        InstallPlan::RunPackageManager { command, .. } => {
            return Err(IhError::Unsupported(format!(
                "apply_appimage ne peut pas exécuter {command} ; une confirmation utilisateur est requise"
            )));
        }
    };

    let staged_metadata = std::fs::metadata(staged).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            IhError::NotFound(format!(
                "artefact préparé introuvable : {}",
                staged.display()
            ))
        } else {
            IhError::Io(err)
        }
    })?;
    if !staged_metadata.is_file() {
        return Err(IhError::NotFound(format!(
            "l'artefact préparé n'est pas un fichier : {}",
            staged.display()
        )));
    }

    let target_metadata = std::fs::metadata(target).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            IhError::NotFound(format!("AppImage cible introuvable : {}", target.display()))
        } else {
            IhError::Io(err)
        }
    })?;
    if !target_metadata.is_file() {
        return Err(IhError::NotFound(format!(
            "la cible n'est pas un fichier : {}",
            target.display()
        )));
    }

    // Empêche de remplacer le fichier préparé par lui-même (y compris via un
    // lien symbolique).
    if std::fs::canonicalize(staged)? == std::fs::canonicalize(target)? {
        return Err(IhError::Update(
            "l'artefact préparé et la cible sont le même fichier".into(),
        ));
    }

    let target_parent = target.parent().ok_or_else(|| {
        IhError::Invalid(format!(
            "la cible n'a pas de dossier parent : {}",
            target.display()
        ))
    })?;
    std::fs::create_dir_all(backup_dir)?;
    if !std::fs::metadata(backup_dir)?.is_dir() {
        return Err(IhError::Invalid(format!(
            "le dossier de sauvegarde n'est pas un dossier : {}",
            backup_dir.display()
        )));
    }

    let backup = unique_backup_path(backup_dir, target)?;
    let old_permissions = target_metadata.permissions();

    std::fs::copy(target, &backup)?;
    std::fs::set_permissions(&backup, old_permissions.clone())?;
    let backup_metadata = std::fs::metadata(&backup)?;
    if backup_metadata.len() != target_metadata.len() {
        let _ = std::fs::remove_file(&backup);
        return Err(IhError::Update(format!(
            "copie de sauvegarde incomplète pour {}",
            target.display()
        )));
    }

    let temp = copy_to_temp(staged, target_parent)?;
    temp.as_file()
        .set_permissions(executable_permissions(old_permissions))?;
    temp.as_file().sync_all()?;
    temp.persist(target).map_err(|err| IhError::Io(err.error))?;

    tracing::info!(
        target = %target.display(),
        backup = %backup.display(),
        "AppImage remplacée, ancienne version sauvegardée"
    );
    Ok(backup)
}

/// Restaure une AppImage depuis une sauvegarde, par copie puis `rename`
/// atomique dans le dossier de la cible.
pub fn rollback_appimage(backup_path: &Path, target: &Path) -> IhResult<()> {
    let backup_metadata = std::fs::metadata(backup_path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            IhError::NotFound(format!(
                "sauvegarde introuvable : {}",
                backup_path.display()
            ))
        } else {
            IhError::Io(err)
        }
    })?;
    if !backup_metadata.is_file() {
        return Err(IhError::NotFound(format!(
            "la sauvegarde n'est pas un fichier : {}",
            backup_path.display()
        )));
    }

    let target_parent = target.parent().ok_or_else(|| {
        IhError::Invalid(format!(
            "la cible n'a pas de dossier parent : {}",
            target.display()
        ))
    })?;
    std::fs::create_dir_all(target_parent)?;

    let temp = copy_to_temp(backup_path, target_parent)?;
    temp.as_file()
        .set_permissions(backup_metadata.permissions())?;
    temp.as_file().sync_all()?;
    temp.persist(target).map_err(|err| IhError::Io(err.error))?;

    tracing::info!(
        target = %target.display(),
        backup = %backup_path.display(),
        "AppImage restaurée depuis la sauvegarde"
    );
    Ok(())
}

fn staged_file_name(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::AppImage => "ih-update.appimage",
        ArtifactKind::Deb => "ih-update.deb",
        ArtifactKind::Pacman => "ih-update.pkg.tar.zst",
    }
}

fn staged_arg(path: &Path) -> IhResult<String> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        IhError::Update(format!(
            "chemin d'artefact non UTF-8, impossible à passer au gestionnaire de paquets : {}",
            path.display()
        ))
    })
}

fn unique_backup_path(backup_dir: &Path, target: &Path) -> IhResult<PathBuf> {
    let file_name = target.file_name().ok_or_else(|| {
        IhError::Invalid(format!(
            "la cible n'a pas de nom de fichier : {}",
            target.display()
        ))
    })?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.6fZ").to_string();
    // Nonce aléatoire en plus de l'horodatage : deux mises à jour concurrentes
    // lancées dans la même microseconde ne se disputent pas le même nom.
    let nonce: u32 = rand::thread_rng().gen();

    for attempt in 0..1_000 {
        let mut name = OsString::from(file_name);
        name.push(format!(".{stamp}-{nonce:08x}"));
        if attempt > 0 {
            name.push(format!("-{attempt}"));
        }
        name.push(".bak");
        let candidate = backup_dir.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(IhError::Update(
        "impossible de choisir un nom de sauvegarde unique".into(),
    ))
}

fn copy_to_temp(source: &Path, directory: &Path) -> IhResult<tempfile::NamedTempFile> {
    let mut source_file = std::fs::File::open(source)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".ih-update-")
        .tempfile_in(directory)?;
    io::copy(&mut source_file, temp.as_file_mut())?;
    Ok(temp)
}

#[cfg(unix)]
fn executable_permissions(base: std::fs::Permissions) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode((base.mode() & 0o777) | 0o111)
}

#[cfg(not(unix))]
fn executable_permissions(base: std::fs::Permissions) -> std::fs::Permissions {
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::verify_signature;
    use crate::test_support::{
        manifest_json, public_key_hex, sha256_hex, sign_hex, test_signing_key, TestServer,
    };
    use std::collections::BTreeMap;

    fn artifact(kind: ArtifactKind, url: &str, digest: &str) -> Artifact {
        Artifact {
            url: url.into(),
            sha256: digest.into(),
            size: None,
            kind,
        }
    }

    fn staged(kind: ArtifactKind, path: PathBuf) -> StagedUpdate {
        let manifest = UpdateManifest {
            schema: 1,
            version: "0.2.0".into(),
            channel: "stable".into(),
            notes: String::new(),
            released_at: None,
            platforms: BTreeMap::new(),
        };
        StagedUpdate {
            manifest,
            artifact: artifact(kind, "https://example.invalid/IH", &"a".repeat(64)),
            path,
        }
    }

    #[test]
    fn plan_appimage_remplace_en_place() {
        let dir = tempfile::tempdir().unwrap();
        let staged_path = dir.path().join("ih-update.appimage");
        std::fs::write(&staged_path, b"nouveau").unwrap();
        let current = dir.path().join("IH.AppImage");

        let plan = plan_install(
            &staged(ArtifactKind::AppImage, staged_path.clone()),
            &current,
        )
        .unwrap();
        assert_eq!(
            plan,
            InstallPlan::ReplaceAppImage {
                target: current,
                staged: staged_path
            }
        );
    }

    #[test]
    fn plan_deb_utilise_pkexec_apt_get_sans_executer() {
        let dir = tempfile::tempdir().unwrap();
        let staged_path = dir.path().join("ih-update.deb");
        std::fs::write(&staged_path, b"paquet").unwrap();

        let plan = plan_install(
            &staged(ArtifactKind::Deb, staged_path.clone()),
            Path::new("/usr/bin/ih"),
        )
        .unwrap();
        assert_eq!(
            plan,
            InstallPlan::RunPackageManager {
                command: "pkexec".into(),
                args: vec![
                    "apt-get".into(),
                    "install".into(),
                    "-y".into(),
                    staged_path.to_string_lossy().into_owned()
                ],
            }
        );
        // Le fichier n'a pas été exécuté : son contenu est intact.
        assert_eq!(std::fs::read(&staged_path).unwrap(), b"paquet");
    }

    #[test]
    fn plan_pacman_utilise_pkexec_pacman_u_sans_executer() {
        let dir = tempfile::tempdir().unwrap();
        let staged_path = dir.path().join("IH-0.2.0-1-x86_64.pkg.tar.zst");
        std::fs::write(&staged_path, b"paquet").unwrap();

        let plan = plan_install(
            &staged(ArtifactKind::Pacman, staged_path.clone()),
            Path::new("/usr/bin/ih"),
        )
        .unwrap();
        assert_eq!(
            plan,
            InstallPlan::RunPackageManager {
                command: "pkexec".into(),
                args: vec![
                    "pacman".into(),
                    "-U".into(),
                    "--noconfirm".into(),
                    staged_path.to_string_lossy().into_owned()
                ],
            }
        );
        assert_eq!(std::fs::read(&staged_path).unwrap(), b"paquet");
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn applique_puis_restaure_avec_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("IH.AppImage");
        let staged_path = dir.path().join("ih-update.appimage");
        let backup_dir = dir.path().join("backups");
        std::fs::write(&target, b"ancien binaire").unwrap();
        std::fs::set_permissions(&target, std::os::unix::fs::PermissionsExt::from_mode(0o750))
            .unwrap();
        std::fs::write(&staged_path, b"nouveau binaire").unwrap();
        std::fs::set_permissions(
            &staged_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .unwrap();

        let update = staged(ArtifactKind::AppImage, staged_path);
        let plan = plan_install(&update, &target).unwrap();
        let backup = apply_appimage(&plan, &backup_dir).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"nouveau binaire");
        assert_ne!(
            mode(&target) & 0o111,
            0,
            "le nouveau binaire doit être exécutable"
        );
        assert_eq!(mode(&target) & 0o700, 0o700);
        assert_eq!(std::fs::read(&backup).unwrap(), b"ancien binaire");
        assert_eq!(mode(&backup), 0o750);
        assert!(backup.starts_with(&backup_dir));

        rollback_appimage(&backup, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"ancien binaire");
        assert_eq!(mode(&target), 0o750);
        assert!(
            backup.exists(),
            "la sauvegarde est conservée après restauration"
        );
    }

    #[test]
    fn apply_refuse_un_plan_paquet_sans_executer() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("IH.AppImage");
        std::fs::write(&target, b"intact").unwrap();

        let plan = InstallPlan::RunPackageManager {
            command: "pkexec".into(),
            args: vec![
                "apt-get".into(),
                "install".into(),
                "-y".into(),
                "x.deb".into(),
            ],
        };
        let err = apply_appimage(&plan, &dir.path().join("backups")).unwrap_err();
        assert!(err.to_string().contains("confirmation"));

        assert_eq!(std::fs::read(&target).unwrap(), b"intact");
    }

    #[test]
    fn apply_sans_cible_echoue_sans_rien_creer() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("absent.AppImage");
        let staged_path = dir.path().join("ih-update.appimage");
        std::fs::write(&staged_path, b"nouveau").unwrap();
        let update = staged(ArtifactKind::AppImage, staged_path);
        let plan = plan_install(&update, &target).unwrap();

        assert!(apply_appimage(&plan, &dir.path().join("backups")).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn rollback_sans_sauvegarde_echoue() {
        let dir = tempfile::tempdir().unwrap();
        assert!(rollback_appimage(
            &dir.path().join("absente.bak"),
            &dir.path().join("IH.AppImage")
        )
        .is_err());
    }

    #[tokio::test]
    async fn preparation_bout_en_bout_avec_manifeste_signe() {
        let artifact_bytes = b"appimage signee de bout en bout".to_vec();
        let digest = sha256_hex(&artifact_bytes);
        let key = test_signing_key();
        let public_key = public_key_hex(&key);

        let server = TestServer::new(vec![]);
        let manifest_bytes = manifest_json("0.2.0", &server.url("/IH.AppImage"), &digest);
        let signature = sign_hex(&key, &manifest_bytes);
        // Le serveur local sert le manifeste signé et l'artefact. Le parseur de
        // production exige HTTPS : le manifeste local est donc construit à
        // partir des octets signés, dont la signature est bien vérifiée.
        verify_signature(&manifest_bytes, &signature, &public_key).unwrap();
        server.set_route("/manifest.json", manifest_bytes.clone());
        server.set_route("/manifest.json.sig", signature.into_bytes());
        server.set_route("/IH.AppImage", artifact_bytes.clone());
        let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes).unwrap();

        let client = UpdateClient::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let staged_update =
            StagedUpdate::prepare(&client, &manifest, ArtifactKind::AppImage, staging.path())
                .await
                .unwrap();

        assert_eq!(staged_update.manifest.version, "0.2.0");
        assert_eq!(staged_update.artifact.kind, ArtifactKind::AppImage);
        assert!(staged_update.path.starts_with(staging.path()));
        assert_eq!(std::fs::read(&staged_update.path).unwrap(), artifact_bytes);
        verify_digest(&staged_update.path, &digest).unwrap();
        assert!(staged_update.path.exists());
    }

    #[tokio::test]
    async fn preparation_refuse_un_digest_invalide() {
        let artifact_bytes = b"contenu".to_vec();
        let key = test_signing_key();
        let public_key = public_key_hex(&key);

        let server = TestServer::new(vec![]);
        let manifest_bytes = manifest_json("0.2.0", &server.url("/IH.AppImage"), &"0".repeat(64));
        let signature = sign_hex(&key, &manifest_bytes);
        verify_signature(&manifest_bytes, &signature, &public_key).unwrap();
        server.set_route("/IH.AppImage", artifact_bytes);
        let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes).unwrap();

        let client = UpdateClient::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        assert!(
            StagedUpdate::prepare(&client, &manifest, ArtifactKind::AppImage, staging.path())
                .await
                .is_err()
        );
        assert!(!staging.path().join("ih-update.appimage").exists());
    }

    #[tokio::test]
    async fn preparation_demande_une_plateforme_presente() {
        let manifest = UpdateManifest {
            schema: 1,
            version: "0.2.0".into(),
            channel: "stable".into(),
            notes: String::new(),
            released_at: None,
            platforms: BTreeMap::new(),
        };
        let client = UpdateClient::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        assert!(
            StagedUpdate::prepare(&client, &manifest, ArtifactKind::Deb, staging.path())
                .await
                .is_err()
        );
    }
}
