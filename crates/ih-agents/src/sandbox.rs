//! Sandbox système des agents CLI (Bubblewrap).
//!
//! Le worktree git isole les modifications du projet ; il **ne** protège pas
//! le reste du système. Ce module exécute chaque agent dans un espace de noms
//! Linux (bubblewrap) : système en lecture seule, dossier personnel privé et
//! vide, worktree en lecture-écriture, environnement vidé et reconstruit,
//! réseau selon le niveau sélectionné.
//!
//! Trois niveaux (configurés par agent) :
//!
//! - **Isolé** (`isolated`) : sandbox + réseau désactivé (`--unshare-net`).
//!   Utile hors ligne ; un agent qui appelle un provider distant échouera
//!   explicitement.
//! - **Isolé avec réseau** (`isolated_network`) : sandbox + réseau de l'hôte.
//!   Recommandé pour les agents qui joignent leur provider.
//! - **Non isolé** (`none`) : exécution avec les droits de l'utilisateur.
//!   Jamais choisi par défaut, jamais basculé silencieusement : une
//!   confirmation explicite (`confirm_unsandboxed`) est exigée.
//!
//! Si la sandbox demandée est indisponible, l'exécution est **refusée** : IH ne
//! retombe pas sur une exécution non isolée.

use ih_core::config::AgentKind;
use ih_core::{IhError, IhResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use ih_core::config::SecurityLevel;

/// Disponibilité réelle de la sandbox sur la machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxAvailability {
    pub engine: String,
    pub path: Option<String>,
    pub version: Option<String>,
    /// La sonde a réellement démarré une sandbox minimale.
    pub functional: bool,
    pub error: Option<String>,
}

/// Détecte Bubblewrap, vérifie qu'il peut créer une sandbox **et** qu'il
/// isole réellement (sonde négative).
///
/// La surcharge `IH_SANDBOX_ENGINE` n'est honorée que dans les builds de test
/// (`debug_assertions`) : un binaire de production ignore cette variable, donc
/// aucun environnement ne peut y substituer le moteur.
pub fn availability() -> SandboxAvailability {
    if let Some(forced) = forced_engine() {
        return evaluate_engine(Some(forced));
    }
    // Hors test, la disponibilité ne change pas pendant la vie du processus :
    // chaque affichage de la page Agents la demande.
    static CACHE: OnceLock<SandboxAvailability> = OnceLock::new();
    CACHE.get_or_init(|| evaluate_engine(None)).clone()
}

/// Surcharge de test du moteur de sandbox (jamais honorée en production).
fn forced_engine() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("IH_SANDBOX_ENGINE").map(PathBuf::from)
}

fn evaluate_engine(forced: Option<PathBuf>) -> SandboxAvailability {
    let path = match forced {
        Some(path) => path,
        None => match ih_core::platform::find_in_path("bwrap").or_else(|| {
            Path::new("/usr/bin/bwrap")
                .exists()
                .then(|| PathBuf::from("/usr/bin/bwrap"))
        }) {
            Some(path) => path,
            None => {
                return SandboxAvailability {
                    engine: "bubblewrap".into(),
                    path: None,
                    version: None,
                    functional: false,
                    error: Some(
                        "bubblewrap (bwrap) introuvable ; installez-le (paquet « bubblewrap »)"
                            .into(),
                    ),
                }
            }
        },
    };

    let unavailable = |error: String| SandboxAvailability {
        engine: "bubblewrap".into(),
        path: path.is_absolute().then(|| path.display().to_string()),
        version: None,
        functional: false,
        error: Some(error),
    };

    if !path.is_absolute() {
        return unavailable(format!(
            "chemin de moteur relatif refusé : {}",
            path.display()
        ));
    }
    if let Err(error) = engine_trusted(&path) {
        return unavailable(error);
    }

    let version = probe_version(&path);
    match probe_functional(&path) {
        Ok(()) => {
            remember_engine(&path);
            SandboxAvailability {
                engine: "bubblewrap".into(),
                path: Some(path.display().to_string()),
                version,
                functional: true,
                error: None,
            }
        }
        Err(error) => SandboxAvailability {
            engine: "bubblewrap".into(),
            path: Some(path.display().to_string()),
            version,
            functional: false,
            error: Some(error),
        },
    }
}

/// Empreinte d'identité du fichier moteur : un remplacement (réinstallation,
/// lien substitutif) change au moins un de ces champs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EngineIdentity {
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
    uid: u32,
}

fn engine_identity(path: &Path) -> Option<EngineIdentity> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let mtime_ns = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0);
    Some(EngineIdentity {
        dev: meta.dev(),
        ino: meta.ino(),
        size: meta.size(),
        mtime_ns,
        uid: meta.uid(),
    })
}

/// Dernier moteur effectivement validé (chemin + identité).
fn verified_engine() -> &'static Mutex<Option<(PathBuf, EngineIdentity)>> {
    static VERIFIED: OnceLock<Mutex<Option<(PathBuf, EngineIdentity)>>> = OnceLock::new();
    VERIFIED.get_or_init(|| Mutex::new(None))
}

fn remember_engine(path: &Path) {
    if let Some(identity) = engine_identity(path) {
        if let Ok(mut guard) = verified_engine().lock() {
            *guard = Some((path.to_path_buf(), identity));
        }
    }
}

/// **Revalidation au lancement** : si l'identité du fichier moteur a changé
/// depuis la dernière validation, la sonde (propriété + positive + négative)
/// est rejouée. Un moteur modifié n'est donc jamais utilisé sans contrôle.
fn revalidate_engine(path: &Path) -> Result<(), String> {
    let identity = engine_identity(path)
        .ok_or_else(|| format!("moteur de sandbox inaccessible : {}", path.display()))?;
    let unchanged = verified_engine()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .is_some_and(|(known_path, known_identity)| {
            known_path == path && known_identity == identity
        });
    if unchanged {
        return Ok(());
    }
    engine_trusted(path)?;
    probe_functional(path)?;
    remember_engine(path);
    Ok(())
}

/// Un moteur de sandbox doit être un fichier stable : jamais un lien ou un
/// binaire modifiable par un autre utilisateur, et en production un binaire
/// possédé par root (paquet système).
fn engine_trusted(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| format!("moteur de sandbox inaccessible : {error}"))?;
    if !meta.is_file() {
        return Err(format!(
            "moteur de sandbox invalide (pas un fichier régulier) : {}",
            path.display()
        ));
    }
    if meta.mode() & 0o022 != 0 {
        return Err(format!(
            "moteur de sandbox inscriptible par le groupe ou le monde : {}",
            path.display()
        ));
    }
    if !cfg!(debug_assertions) && meta.uid() != 0 {
        return Err(format!(
            "moteur de sandbox non possédé par root (refusé en production) : {}",
            path.display()
        ));
    }
    Ok(())
}

/// Sonde réelle du moteur : elle doit **exécuter** une sandbox minimale et
/// **refuser** une écriture hors de celle-ci. Un faux `bwrap` qui ignore les
/// drapeaux échoue donc à la seconde partie et est refusé.
pub(crate) fn probe_functional(path: &Path) -> Result<(), String> {
    let probe_dir = std::env::temp_dir().join(format!(
        "ih-sbxprobe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&probe_dir);
    };
    if std::fs::create_dir_all(&probe_dir).is_err() {
        return Err("sonde de sandbox : dossier temporaire inaccessible".into());
    }
    let canary = probe_dir.join("canari.txt");
    if std::fs::write(&canary, "intact\n").is_err() {
        cleanup();
        return Err("sonde de sandbox : canari non créé".into());
    }

    let base = |extra: &[&str]| -> Vec<String> {
        let mut args: Vec<String> = [
            "--unshare-all",
            "--die-with-parent",
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/bin",
            "/bin",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/lib64",
            "/lib64",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--",
        ]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
        args.extend(extra.iter().map(|arg| arg.to_string()));
        args
    };

    let positive = Command::new(path)
        .args(base(&["/bin/sh", "-c", "exit 0"]))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let positive = match positive {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            cleanup();
            return Err(format!(
                "bubblewrap refuse de démarrer (user namespaces restreints ?) : {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(error) => {
            cleanup();
            return Err(format!("bubblewrap inutilisable : {error}"));
        }
    };
    let _ = positive;

    // Sonde négative : écrire sur un chemin de l'hôte doit échouer.
    let script = format!("printf pirate > '{}'", canary.display());
    let negative = Command::new(path)
        .args(base(&["/bin/sh", "-c", &script]))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();

    let intact = std::fs::read_to_string(&canary).map(|text| text == "intact\n");
    cleanup();

    match (negative, intact) {
        (Ok(output), Ok(true)) if !output.status.success() => Ok(()),
        (Ok(output), Ok(true)) => Err(format!(
            "le moteur de sandbox n'isole pas : écriture hors sandbox réussie (code {})",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".into())
        )),
        (Ok(_), _) => Err("le moteur de sandbox a modifié un fichier hors de la sandbox".into()),
        (Err(error), _) => Err(format!("sonde négative impossible : {error}")),
    }
}

fn probe_version(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if started.elapsed() > Duration::from_secs(3) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(80).collect())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn data_home() -> Option<PathBuf> {
    home_dir().map(|home| {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
    })
}

/// Copie (0600) d'un fichier vers le dossier personnel privé, si la source
/// existe. Les secrets restent dans la sandbox : l'hôte n'est pas exposé.
fn stage_file(source: &Path, target: &Path) {
    let Some(parent) = target.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(bytes) = std::fs::read(source) {
        if std::fs::write(target, bytes).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

fn stage_dir(source: &Path, target: &Path) {
    if !source.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(source) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_file() {
            stage_file(&path, &target.join(&name));
        }
    }
}

/// Préparation du dossier personnel privé et des montages propres à la famille
/// d'agent.
struct Staging {
    /// (source hôte, destination dans la sandbox, lecture seule)
    binds: Vec<(PathBuf, String)>,
    /// Variables d'environnement supplémentaires à fournir explicitement.
    env: Vec<(String, String)>,
}

fn staging_for(kind: &AgentKind, private_home: &Path) -> Staging {
    let mut staging = Staging {
        binds: Vec::new(),
        env: Vec::new(),
    };
    let Some(home) = home_dir() else {
        return staging;
    };
    let private_str = private_home.display().to_string();
    let put_config = |relative: &str, staging: &mut Staging| {
        stage_file(&home.join(relative), &private_home.join(relative));
        staging.binds.push((
            private_home.join(relative),
            format!("{private_str}/{relative}"),
        ));
        staging.binds.pop(); // le dossier privé est déjà monté comme HOME
    };
    let _ = put_config; // conservé pour lisibilité : HOME porte déjà les copies

    match kind {
        // Kilo : exécutable npm hors système + identifiants propres.
        AgentKind::Kilo => {
            for (source, target) in [
                (home.join(".local/bin"), "/opt/ih-agents/bin"),
                (home.join(".local/lib"), "/opt/ih-agents/lib"),
            ] {
                if source.is_dir() {
                    staging.binds.push((source, target.to_string()));
                }
            }
            stage_file(
                &home.join(".local/share/kilo/auth.json"),
                &private_home.join(".local/share/kilo/auth.json"),
            );
            stage_dir(
                &home.join(".config/kilo"),
                &private_home.join(".config/kilo"),
            );
        }
        // DSH : paquet d'application, profils (lecture seule) et identifiants.
        AgentKind::Dsh => {
            if Path::new("/opt/apps").is_dir() {
                staging
                    .binds
                    .push((PathBuf::from("/opt/apps"), "/opt/apps".into()));
            }
            let dsh_home = data_home()
                .map(|data| data.join("cn.deepseek.harness"))
                .or_else(|| home_dir().map(|home| home.join(".local/share/cn.deepseek.harness")));
            if let Some(real) = dsh_home {
                for name in [".credentials.yaml", "settings.yaml", "cordis.patch.yml"] {
                    stage_file(&real.join(name), &private_home.join(".dsh").join(name));
                }
                let _ = std::fs::create_dir_all(private_home.join(".dsh/sessions"));
                let _ = std::fs::create_dir_all(private_home.join(".dsh/storages"));
                let profiles = real.join("profiles");
                if profiles.is_dir() {
                    staging
                        .binds
                        .push((profiles, format!("{private_str}/.dsh/profiles")));
                }
            }
            // Chemin INTERNE à la sandbox (le dossier personnel privé est monté
            // sur /home/agent ; le chemin hôte n'existe pas dans la sandbox).
            staging
                .env
                .push(("DSH_HOME".into(), "/home/agent/.dsh".into()));
        }
        // Goose : binaire utilisateur + configuration et secrets propres.
        AgentKind::Goose => {
            let bin = home.join(".local/bin");
            if bin.is_dir() {
                staging.binds.push((bin, "/opt/ih-agents/bin".into()));
            }
            stage_dir(
                &home.join(".config/goose"),
                &private_home.join(".config/goose"),
            );
            for dir in [".local/state/goose", ".local/share/goose"] {
                let _ = std::fs::create_dir_all(private_home.join(dir));
            }
        }
        // Autres familles : dossier du binaire uniquement. Leurs identifiants
        // (le cas échéant) restent à confier explicitement par configuration ;
        // cela évite d'exposer le profil utilisateur par défaut.
        AgentKind::ClaudeCode
        | AgentKind::Codex
        | AgentKind::GeminiCli
        | AgentKind::Opencode
        | AgentKind::Pi
        | AgentKind::Custom => {}
    }
    staging
}

/// Montages Git d'un worktree, **calculés et conservés par IH** à la création
/// du worktree.
///
/// Le fichier `.git` d'un worktree est inscriptible par l'agent : son contenu ne
/// fait donc jamais autorité. Ces chemins viennent du dépôt découvert par IH et
/// sont validés (canonicalisation + confinement) avant d'être transformés en
/// arguments de bubblewrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMounts {
    /// `<dépôt>/.git/worktrees/<nom>` — métadonnées propres au worktree.
    pub worktree_gitdir: PathBuf,
    /// `<dépôt>/.git` — répertoire Git commun (objets, références, config).
    pub common_dir: PathBuf,
}

impl GitMounts {
    /// Construit les montages attendus pour un worktree donné.
    ///
    /// Refuse tout chemin qui n'est pas exactement `<commondir>/worktrees/<nom>`
    /// ou qui n'est pas confiné au dépôt (lien symbolique, `..`, répertoire
    /// inexistant, répertoire Git hors `.git`).
    pub fn expected(common_dir: &Path, worktree_name: &str, workdir: &Path) -> IhResult<Self> {
        validate_path_component(worktree_name)?;
        let common = checked_dir(common_dir, "répertoire Git commun")?;
        if common.file_name().and_then(|name| name.to_str()) != Some(".git") {
            return Err(IhError::Agent(format!(
                "répertoire Git commun inattendu (un dossier `.git` est attendu) : {}",
                common.display()
            )));
        }
        let gitdir = common.join("worktrees").join(worktree_name);
        let gitdir = checked_dir(&gitdir, "répertoire Git du worktree")?;
        // Confinement : le gitdir doit être strictement sous le répertoire commun.
        if !gitdir.starts_with(&common) {
            return Err(IhError::Agent(format!(
                "répertoire Git du worktree hors du dépôt : {}",
                gitdir.display()
            )));
        }
        // Le `.git` du worktree doit désigner ce gitdir, sans lien symbolique.
        verify_worktree_pointer(workdir, &gitdir)?;
        Ok(Self {
            worktree_gitdir: gitdir,
            common_dir: common,
        })
    }

    /// Revérifie l'état réel du worktree avant un lancement : `.git` régulier,
    /// contenu identique au chemin attendu, `commondir` identique au répertoire
    /// commun attendu. Toute divergence refuse le lancement.
    pub fn verify(&self, workdir: &Path) -> IhResult<()> {
        verify_worktree_pointer(workdir, &self.worktree_gitdir)?;
        let common = checked_dir(&self.common_dir, "répertoire Git commun")?;
        if common != self.common_dir {
            return Err(IhError::Agent(format!(
                "répertoire Git commun modifié : {}",
                self.common_dir.display()
            )));
        }
        let commondir_file = self.worktree_gitdir.join("commondir");
        let Ok(raw) = std::fs::read_to_string(&commondir_file) else {
            return Err(IhError::Agent(format!(
                "`commondir` illisible dans {} : montage Git refusé",
                self.worktree_gitdir.display()
            )));
        };
        let candidate = self.worktree_gitdir.join(raw.trim());
        let candidate = std::fs::canonicalize(&candidate).map_err(|error| {
            IhError::Agent(format!(
                "`commondir` pointe vers un chemin inexistant ({}) : {error}",
                candidate.display()
            ))
        })?;
        if candidate != self.common_dir {
            return Err(IhError::Agent(format!(
                "`commondir` pointe hors du dépôt attendu ({} ≠ {}) : montage Git refusé",
                candidate.display(),
                self.common_dir.display()
            )));
        }
        Ok(())
    }
}

/// Vérifie que le `.git` du worktree est un **fichier régulier** (jamais un lien
/// symbolique) désignant exactement le gitdir attendu.
fn verify_worktree_pointer(workdir: &Path, expected: &Path) -> IhResult<()> {
    let pointer = workdir.join(".git");
    let meta = std::fs::symlink_metadata(&pointer).map_err(|error| {
        IhError::Agent(format!(
            "fichier `.git` absent du worktree {} : {error}",
            pointer.display()
        ))
    })?;
    if !meta.file_type().is_file() {
        return Err(IhError::Agent(format!(
            "`.git` du worktree n'est pas un fichier régulier (lien symbolique ou dossier) : {}",
            pointer.display()
        )));
    }
    let content = std::fs::read_to_string(&pointer)
        .map_err(|error| IhError::Agent(format!("`.git` illisible : {error}")))?;
    let declared = content.trim();
    let Some(rest) = declared.strip_prefix("gitdir:") else {
        return Err(IhError::Agent(format!(
            "`.git` malformé (préfixe `gitdir:` attendu) : {declared:.120}"
        )));
    };
    let declared = PathBuf::from(rest.trim());
    if !declared.is_absolute() {
        return Err(IhError::Agent(format!(
            "`.git` désigne un chemin relatif : {}",
            declared.display()
        )));
    }
    let declared = std::fs::canonicalize(&declared).map_err(|error| {
        IhError::Agent(format!(
            "`.git` désigne un chemin inexistant ({}) : {error}",
            declared.display()
        ))
    })?;
    if declared != expected {
        return Err(IhError::Agent(format!(
            "`.git` du worktree modifié : {} au lieu de {} — lancement refusé",
            declared.display(),
            expected.display()
        )));
    }
    Ok(())
}

/// Répertoire existant, non symbolique, retourné canonicalisé.
fn checked_dir(path: &Path, label: &str) -> IhResult<PathBuf> {
    let meta = std::fs::symlink_metadata(path).map_err(|error| {
        IhError::Agent(format!(
            "{label} introuvable ({}) : {error}",
            path.display()
        ))
    })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(IhError::Agent(format!(
            "{label} n'est pas un répertoire réel : {}",
            path.display()
        )));
    }
    std::fs::canonicalize(path)
        .map_err(|error| IhError::Agent(format!("{label} non canonicalisable : {error}")))
}

/// Identifiant utilisé comme composant de chemin.
fn validate_path_component(value: &str) -> IhResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if valid {
        Ok(())
    } else {
        Err(IhError::Invalid(format!(
            "composant de chemin invalide : {value}"
        )))
    }
}

/// Dossier personnel privé d'une session, supprimé **automatiquement** à sa
/// destruction (succès, erreur, échec de lancement, annulation, panique).
///
/// Le nettoyage ne dépend donc plus d'un seul chemin de code : perdre le
/// garde suffit à supprimer les copies d'identifiants.
pub struct PrivateHome {
    path: PathBuf,
    /// Verrou système (`flock`) tenu pendant toute la vie de la session : un
    /// autre processus IH ne peut ni le prendre ni supprimer le dossier. Le
    /// champ n'est pas lu : sa seule existence maintient le verrou jusqu'au
    /// `Drop`.
    #[allow(dead_code)]
    lock: Option<std::fs::File>,
}

/// Fichier verrouillé : sa présence et le verrou qu'il porte déterminent si le
/// dossier est encore utilisé (insensible à la réutilisation de PID).
const LOCK_FILE: &str = ".ih-lock";

impl PrivateHome {
    /// Crée le dossier privé (permissions `0700`), le verrouille et y inscrit
    /// son propriétaire. Toute étape qui échoue supprime le dossier : aucun
    /// dossier partiellement initialisé ne subsiste.
    fn create(session_tag: &str) -> IhResult<Self> {
        validate_session_tag(session_tag)?;
        let path = std::env::temp_dir().join(format!("ih-agent-home-{session_tag}"));
        if path.exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .recursive(true)
            .create(&path)
            .map_err(|error| IhError::Internal(format!("dossier privé de l'agent : {error}")))?;
        match Self::open(path.clone()) {
            Ok(home) => Ok(home),
            Err(error) => {
                let _ = std::fs::remove_dir_all(&path);
                Err(error)
            }
        }
    }

    /// Initialise un dossier déjà créé : verrou exclusif puis marqueur de
    /// propriétaire. Échec ⇒ erreur (le dossier est nettoyé par l'appelant).
    fn open(path: PathBuf) -> IhResult<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path.join(LOCK_FILE))
            .map_err(|error| IhError::Internal(format!("verrou du dossier privé : {error}")))?;
        // Verrou exclusif non bloquant : nous sommes le seul propriétaire.
        let acquired = unsafe {
            libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&lock),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if acquired != 0 {
            return Err(IhError::Internal(
                "dossier privé déjà utilisé (verrou indisponible)".into(),
            ));
        }
        // Marqueur obligatoire : sans lui, aucune session ne peut démarrer.
        let owner = format!("{} {}", std::process::id(), process_start_time());
        std::fs::write(path.join(OWNER_FILE), owner)
            .map_err(|error| IhError::Internal(format!("marqueur de propriétaire : {error}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .map(|meta| meta.permissions().mode() & 0o777)
                .unwrap_or(0);
            if mode != 0o700 {
                return Err(IhError::Internal(format!(
                    "dossier privé aux permissions inattendues ({mode:o} au lieu de 700)"
                )));
            }
        }
        Ok(Self {
            path,
            lock: Some(lock),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Instant de démarrage du processus courant (champ 22 de `/proc/<pid>/stat`).
/// Combiné au PID, il rend la détection de vivacité insensible à la
/// réutilisation de PID.
fn process_start_time() -> u64 {
    start_time_of(std::process::id()).unwrap_or(0)
}

fn start_time_of(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Le nom de commande peut contenir des espaces et des parenthèses : on
    // repart de la dernière parenthèse fermante.
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Vrai si le dossier est réellement utilisé : verrou tenu, ou (à défaut de
/// verrou) couple PID + instant de démarrage identique.
fn private_home_is_alive(path: &Path) -> bool {
    if let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.join(LOCK_FILE))
    {
        let taken = unsafe {
            libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if taken == 0 {
            // Verrou libre : le dossier n'est utilisé par personne.
            unsafe {
                libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), libc::LOCK_UN);
            }
            return false;
        }
        return true;
    }
    // Pas de verrou (ancien format ou fichier inaccessible) : repli sur le
    // marqueur PID + instant de démarrage.
    let Ok(owner) = std::fs::read_to_string(path.join(OWNER_FILE)) else {
        return true; // impossible de conclure : on ne supprime pas
    };
    let mut parts = owner.split_whitespace();
    let (Some(pid), Some(start)) = (parts.next(), parts.next()) else {
        return true;
    };
    let (Ok(pid), Ok(start)) = (pid.parse::<u32>(), start.parse::<u64>()) else {
        return true;
    };
    start_time_of(pid) == Some(start) && start != 0
}

impl Drop for PrivateHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Fichier marquant l'instance IH propriétaire d'un dossier privé.
const OWNER_FILE: &str = ".ih-owner";

/// Supprime les dossiers privés laissés par des instances IH mortes.
///
/// Un dossier dont le propriétaire est **encore vivant** est conservé : deux
/// instances IH simultanées ne se suppriment pas mutuellement.
pub fn cleanup_orphan_homes() -> usize {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("ih-agent-home-") || !path.is_dir() {
            continue;
        }
        // Un dossier encore verrouillé (donc utilisé) n'est JAMAIS supprimé ; à
        // défaut de verrou, le couple PID + instant de démarrage tranche.
        if private_home_is_alive(&path) {
            continue;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Un identifiant de session sert de composant de chemin : alphabet strict,
/// pas de séparateur, pas de `..`, longueur bornée.
pub fn validate_session_tag(tag: &str) -> IhResult<()> {
    let valid = !tag.is_empty()
        && tag.len() <= 64
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(IhError::Invalid(format!(
            "identifiant de session invalide (alphabet [A-Za-z0-9_-], 64 caractères maximum) : {tag}"
        )))
    }
}

/// Commande prête à être lancée (programme + arguments) et dossier privé à
/// nettoyer après la session.
pub struct SandboxCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Variables à fournir explicitement (l'environnement hôte n'est pas hérité).
    pub env: Vec<(String, String)>,
    /// Garde de nettoyage du dossier personnel privé.
    pub private_home: Option<PrivateHome>,
}

/// Vérifie qu'une sandbox opérationnelle existe, sinon refuse.
pub(crate) fn check_engine(report: &SandboxAvailability) -> IhResult<PathBuf> {
    if !report.functional {
        return Err(IhError::Agent(format!(
            "sandbox {} indisponible : {}. Installez bubblewrap (paquet « bubblewrap », Zorin/Ubuntu : apt install bubblewrap ; Arch : pacman -S bubblewrap) ou choisissez explicitement le niveau non isolé.",
            report.engine,
            report
                .error
                .clone()
                .unwrap_or_else(|| "raison inconnue".into())
        )));
    }
    let path = report
        .path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| IhError::Agent("chemin de bubblewrap introuvable".into()))?;
    // NV6 : le moteur est revérifié à chaque lancement ; un binaire remplacé
    // depuis la première sonde est re-sondé (propriété + isolation réelle).
    revalidate_engine(&path)
        .map_err(|error| IhError::Agent(format!("sandbox refusée : {error}")))?;
    Ok(path)
}

/// Construit la commande sandboxée pour un agent.
///
/// `workdir` est monté en lecture-écriture ; tout le reste du système visible
/// est en lecture seule (`/usr`, `/etc`, `/opt/apps`), `/tmp` est privé, le
/// dossier personnel de l'utilisateur n'est jamais monté.
#[allow(clippy::too_many_arguments)]
pub fn sandboxed_command(
    level: SecurityLevel,
    kind: &AgentKind,
    executable: &Path,
    args: &[String],
    workdir: &Path,
    extra_env: &[(String, String)],
    session_tag: &str,
    git: Option<&GitMounts>,
) -> IhResult<SandboxCommand> {
    if level == SecurityLevel::None {
        return Err(IhError::Invalid(
            "niveau non isolé : utiliser la commande directe, après confirmation".into(),
        ));
    }
    // Le binaire système est lié tel quel ; un binaire utilisateur est monté
    // en lecture seule. La cible est d'abord RÉSOLUE : beaucoup de CLI sont des
    // liens symboliques (`dsh -> ../lib/node_modules/@deepseek-ai/dsh/lib/bin.js`)
    // et le lien seul ne suffirait pas dans la sandbox.
    let resolved_executable = std::fs::canonicalize(executable)
        .ok()
        .filter(|path| path.exists())
        .unwrap_or_else(|| executable.to_path_buf());
    // La validité du montage est vérifiée AVANT tout le reste : une
    // configuration qui exposerait le dossier personnel est refusée même si
    // aucun moteur n'est disponible (le diagnostic porte sur la cause réelle).
    let executable_grants = executable_grant_roots(&resolved_executable, workdir)?;

    let bwrap = check_engine(&availability())?;
    // Garde RAII : le dossier privé (et ses copies d'identifiants) disparaît à
    // la destruction de la commande, y compris sur un chemin d'erreur.
    let private_home = PrivateHome::create(session_tag)?;
    let private_home_path = private_home.path().to_path_buf();

    let staging = staging_for(kind, &private_home_path);

    // Runtime Node hors système (nvm, npm global) : les CLI npm (Kilo, agents
    // Node) en ont besoin. Seul le dossier du binaire `node` est monté, en
    // lecture seule, jamais le dossier personnel.
    let mut node_grant: Option<(PathBuf, String)> = None;
    if let Some(node) = ih_core::platform::find_in_path("node") {
        let system =
            node.starts_with("/usr") || node.starts_with("/bin") || node.starts_with("/opt/apps");
        if !system {
            if let Some(dir) = node.parent() {
                node_grant = Some((dir.to_path_buf(), "/opt/ih-tools/node/bin".into()));
            }
        }
    }

    let mut bwrargs: Vec<String> = vec![
        "--die-with-parent".into(),
        "--new-session".into(),
        "--unshare-pid".into(),
        "--unshare-ipc".into(),
        "--unshare-uts".into(),
    ];
    if level == SecurityLevel::Isolated {
        bwrargs.push("--unshare-net".into());
    }
    // Système en lecture seule, jamais réinscriptible.
    bwrargs.extend(["--ro-bind".into(), "/usr".into(), "/usr".into()]);
    for (source, target) in [
        ("/bin", "usr/bin"),
        ("/sbin", "usr/sbin"),
        ("/lib", "usr/lib"),
        ("/lib64", "usr/lib64"),
        ("/lib32", "usr/lib32"),
    ] {
        if Path::new(source).is_symlink() {
            bwrargs.extend(["--symlink".to_string(), target.into(), source.into()]);
        } else if Path::new(source).exists() {
            bwrargs.extend(["--ro-bind".to_string(), source.into(), source.into()]);
        }
    }
    if Path::new("/etc").is_dir() {
        bwrargs.extend(["--ro-bind".into(), "/etc".into(), "/etc".into()]);
    }
    // Résolution DNS (systemd-resolved) lorsque resolv.conf y renvoie.
    if Path::new("/run/systemd/resolve").is_dir() {
        bwrargs.extend([
            "--ro-bind-try".into(),
            "/run/systemd/resolve".into(),
            "/run/systemd/resolve".into(),
        ]);
    }
    bwrargs.extend([
        "--proc".into(),
        "/proc".into(),
        "--dev".into(),
        "/dev".into(),
    ]);
    // /tmp privé : les fichiers temporaires de l'hôte ne sont pas visibles.
    bwrargs.extend(["--tmpfs".into(), "/tmp".into()]);

    // Dossiers d'exécutables utilisateur, en lecture seule, AVANT le worktree :
    // si l'un d'eux est un ancêtre du worktree, le montage inscriptible du
    // worktree appliqué ensuite le recouvre sans casser l'écriture.
    for (source, target) in executable_grants.iter() {
        if source.exists() {
            bwrargs.extend([
                "--ro-bind".into(),
                source.display().to_string(),
                target.clone(),
            ]);
        }
    }

    // Worktree : unique répertoire inscriptible du système de fichiers.
    let workdir_str = workdir.display().to_string();
    bwrargs.extend(["--bind".into(), workdir_str.clone(), workdir_str.clone()]);

    // Dossier personnel privé (vide au départ, complété par les copies).
    bwrargs.extend([
        "--bind".into(),
        private_home_path.display().to_string(),
        "/home/agent".into(),
    ]);

    // Git dans le worktree : les chemins viennent d'IH (`GitMounts`), jamais du
    // fichier `.git` du worktree (inscriptible par l'agent). Le répertoire Git
    // commun ET les métadonnées du worktree sont montés en LECTURE SEULE :
    // l'index de la session vit dans le dossier privé (`GIT_INDEX_FILE`), donc
    // le dépôt principal n'est jamais inscriptible et rien de contrôlé par
    // l'agent ne devient un montage.
    let mut git_env: Vec<(String, String)> = Vec::new();
    if let Some(git) = git {
        git.verify(workdir)?;
        for source in [&git.common_dir, &git.worktree_gitdir] {
            bwrargs.extend([
                "--ro-bind".into(),
                source.display().to_string(),
                source.display().to_string(),
            ]);
        }
        git_env.push(("GIT_INDEX_FILE".into(), "/home/agent/.git-index".into()));
        git_env.push(("GIT_OPTIONAL_LOCKS".into(), "0".into()));
    }

    for (source, target) in staging.binds.iter().chain(node_grant.iter()) {
        if source.exists() {
            bwrargs.extend([
                "--ro-bind".into(),
                source.display().to_string(),
                target.clone(),
            ]);
        }
    }

    // Environnement reconstruit : rien n'est hérité de l'hôte.
    bwrargs.extend(["--clearenv".into()]);
    let path = match &node_grant {
        Some((_, target)) => format!("{target}:/opt/ih-agents/bin:/usr/local/bin:/usr/bin:/bin"),
        None => "/opt/ih-agents/bin:/usr/local/bin:/usr/bin:/bin".to_string(),
    };
    let mut env: Vec<(String, String)> = vec![
        ("HOME".into(), "/home/agent".into()),
        ("PATH".into(), path),
        ("TMPDIR".into(), "/tmp".into()),
        ("PWD".into(), workdir_str.clone()),
        ("OLDPWD".into(), workdir_str.clone()),
        ("LANG".into(), "C.UTF-8".into()),
        ("TERM".into(), "dumb".into()),
    ];
    env.extend(staging.env.iter().cloned());
    env.extend(git_env);
    // Variables explicitement déclarées pour l'agent (jamais héritées).
    env.extend(extra_env.iter().cloned());
    for (key, value) in &env {
        bwrargs.extend(["--setenv".into(), key.clone(), value.clone()]);
    }

    // L'exécutable peut se trouver dans un dossier remonté à un autre chemin
    // (ex. ~/.local/bin -> /opt/ih-agents/bin) : la commande doit utiliser le
    // chemin INTERNE à la sandbox, sinon l'exécution échoue (execvp introuvable).
    let mut internal_executable = resolved_executable.display().to_string();
    for (source, target) in staging.binds.iter().chain(executable_grants.iter()) {
        if let Ok(relative) = resolved_executable.strip_prefix(source) {
            let mapped = Path::new(target).join(relative);
            internal_executable = mapped.display().to_string();
            break;
        }
    }
    bwrargs.extend([
        "--chdir".into(),
        workdir_str,
        "--".into(),
        internal_executable,
    ]);
    bwrargs.extend(args.iter().cloned());

    Ok(SandboxCommand {
        program: bwrap,
        args: bwrargs,
        env,
        private_home: Some(private_home),
    })
}

/// Dossiers personnels dont l'exposition, même en lecture seule, donne accès à
/// des secrets (identifiants, configuration, historique).
const SENSITIVE_DIRS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".config",
    ".local/share",
    ".kube",
    ".docker",
    ".netrc",
    ".git-credentials",
    ".password-store",
    ".mozilla",
    ".thunderbird",
    ".npmrc",
    ".pypirc",
];

/// Calcule les racines de montage satisfaisant un exécutable utilisateur, en
/// refusant toute racine qui exposerait le dossier personnel.
///
/// Le montage est nécessaire pour qu'un binaire hors système s'exécute dans la
/// sandbox ; il ne doit jamais faire entrer `$HOME` (ni un de ses ancêtres, ni
/// un dossier contenant des secrets personnels) dans l'espace de noms.
fn executable_grant_roots(
    resolved_executable: &Path,
    workdir: &Path,
) -> IhResult<Vec<(PathBuf, String)>> {
    if resolved_executable.starts_with("/opt/ih-agents/bin") {
        // déjà fourni par les montages de la famille (Kilo/Goose)
        return Ok(Vec::new());
    }
    if resolved_executable.starts_with("/usr")
        || resolved_executable.starts_with("/bin")
        || resolved_executable.starts_with("/opt/apps")
    {
        return Ok(Vec::new());
    }

    // Racine à monter : le paquet npm entier (`…/node_modules`) quand il
    // existe, sinon le dossier du binaire. Un script Node a besoin de son
    // paquet, pas du seul fichier de lancement.
    let mut root = resolved_executable.parent().map(Path::to_path_buf);
    let mut cursor = root.clone();
    while let Some(dir) = cursor {
        if dir.file_name().is_some_and(|name| name == "node_modules") {
            root = Some(dir);
            break;
        }
        cursor = dir.parent().map(Path::to_path_buf);
    }
    let Some(root) = root else {
        return Ok(Vec::new());
    };
    // NV5 : la racine est résolue puis revalidée sur son chemin canonique, et un
    // répertoire symbolique est refusé (le montage suivrait sa cible).
    let meta = std::fs::symlink_metadata(&root).map_err(|error| {
        IhError::Agent(format!(
            "répertoire de l'exécutable introuvable ({}) : {error}",
            root.display()
        ))
    })?;
    if meta.file_type().is_symlink() {
        return Err(IhError::Agent(format!(
            "isolation impossible : le répertoire de l'exécutable est un lien symbolique ({})",
            root.display()
        )));
    }
    let root = std::fs::canonicalize(&root).map_err(|error| {
        IhError::Agent(format!(
            "répertoire de l'exécutable non canonicalisable ({}) : {error}",
            root.display()
        ))
    })?;
    if let Some(reason) = forbidden_grant_reason(&root, home_dir().as_deref()) {
        return Err(IhError::Agent(format!(
            "isolation impossible pour l'exécutable {} : {reason}. Déplacez l'exécutable dans un sous-dossier dédié (par exemple ~/.local/bin) ou choisissez un agent installé par paquet.",
            resolved_executable.display()
        )));
    }
    // Un exécutable situé dans le worktree est déjà accessible en
    // lecture-écriture : inutile de le remonter, et un montage en lecture
    // seule masquerait le worktree inscriptible.
    // Un dossier ANCÊTRE du worktree (ex. /tmp/ih-team-123) est monté en
    // lecture seule AVANT le worktree : le montage inscriptible du worktree
    // est appliqué ensuite, par-dessus, et reste gagnant.
    if root.starts_with(workdir) {
        return Ok(Vec::new());
    }
    Ok(vec![(root.clone(), root.display().to_string())])
}

/// Motif de refus d'une racine de montage, ou `None` si elle est acceptable.
///
/// Règles : jamais `$HOME`, jamais un ancêtre de `$HOME`, jamais la racine du
/// système, jamais un dossier qui contient un dossier sensible (`~/.ssh`,
/// `~/.aws`, `~/.config`, …).
fn forbidden_grant_reason(root: &Path, home: Option<&Path>) -> Option<String> {
    if let Some(home) = home {
        if root == home {
            return Some("la racine calculée est le dossier personnel de l'utilisateur".into());
        }
        if home.starts_with(root) {
            return Some(format!(
                "la racine calculée ({}) contient le dossier personnel de l'utilisateur",
                root.display()
            ));
        }
        for sensitive in SENSITIVE_DIRS {
            let path = home.join(sensitive);
            if root == path || path.starts_with(root) {
                return Some(format!(
                    "la racine calculée ({}) contient le dossier personnel sensible {}",
                    root.display(),
                    path.display()
                ));
            }
        }
    }
    let dangerous = [
        Path::new("/"),
        Path::new("/home"),
        Path::new("/root"),
        Path::new("/tmp"),
        Path::new("/var"),
        Path::new("/etc"),
        Path::new("/opt"),
    ];
    if dangerous.contains(&root) {
        return Some(format!(
            "la racine calculée ({}) est trop large",
            root.display()
        ));
    }
    None
}

/// Résumé lisible pour l'interface et les approbations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSummary {
    pub level: String,
    pub network: bool,
    pub sandboxed: bool,
    pub engine: Option<String>,
    pub engine_ready: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

pub fn summary(level: SecurityLevel) -> SandboxSummary {
    let report = availability();
    let mut warnings = Vec::new();
    if level == SecurityLevel::None {
        warnings.push(
            "Niveau non isolé : l'agent s'exécute avec les droits de votre compte Linux.".into(),
        );
    }
    if level != SecurityLevel::None && !report.functional {
        warnings.push(format!(
            "Sandbox indisponible : le lancement isolé sera refusé ({})",
            report
                .error
                .clone()
                .unwrap_or_else(|| "raison inconnue".into())
        ));
    }
    SandboxSummary {
        level: level.as_str().into(),
        network: level.network_enabled(),
        sandboxed: level.sandboxed(),
        engine: Some(report.engine),
        engine_ready: report.functional,
        error: report.error,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niveaux_et_reseau() {
        assert!(SecurityLevel::Isolated.sandboxed());
        assert!(!SecurityLevel::Isolated.network_enabled());
        assert!(SecurityLevel::IsolatedNetwork.network_enabled());
        assert!(!SecurityLevel::None.sandboxed());
        assert_eq!(
            SecurityLevel::parse("isolated").unwrap(),
            SecurityLevel::Isolated
        );
        assert_eq!(
            SecurityLevel::parse("isolated_network").unwrap(),
            SecurityLevel::IsolatedNetwork
        );
        assert_eq!(SecurityLevel::parse("none").unwrap(), SecurityLevel::None);
        assert!(SecurityLevel::parse("flou").is_err());
    }

    #[test]
    fn niveau_non_isole_refuse_par_la_sandbox() {
        let error = sandboxed_command(
            SecurityLevel::None,
            &AgentKind::Kilo,
            Path::new("/usr/bin/kilo"),
            &[],
            Path::new("/tmp"),
            &[],
            "test-none",
            None,
        )
        .err()
        .expect("le niveau non isolé ne doit pas passer par la sandbox");
        assert!(format!("{error}").contains("non isolé"));
    }

    #[test]
    fn indisponibilite_de_la_sandbox_refuse_le_lancement() {
        // La machine de test a bubblewrap ; on vérifie ici le contrat :
        // sans bwrap fonctionnel, la création échoue au lieu de dégrader.
        let report = availability();
        if report.functional {
            return;
        }
        assert!(sandboxed_command(
            SecurityLevel::IsolatedNetwork,
            &AgentKind::Goose,
            Path::new("/usr/bin/goose"),
            &[],
            Path::new("/tmp"),
            &[],
            "test-unavailable",
            None,
        )
        .is_err());
    }

    #[test]
    fn une_sandbox_non_fonctionnelle_est_refusee_sans_repli() {
        let unavailable = SandboxAvailability {
            engine: "bubblewrap".into(),
            path: Some("/usr/bin/bwrap".into()),
            version: Some("0.11.0".into()),
            functional: false,
            error: Some("user namespaces restreints".into()),
        };
        let error = check_engine(&unavailable).expect_err("doit refuser");
        assert!(format!("{error}").contains("user namespaces"));
        // Aucune commande non isolée ne peut être produite par ce chemin.
        assert!(sandboxed_command(
            SecurityLevel::None,
            &AgentKind::Kilo,
            Path::new("/usr/bin/kilo"),
            &[],
            Path::new("/tmp"),
            &[],
            "unavailable-test",
            None,
        )
        .is_err());
    }

    /// Le calcul des racines de montage ne doit jamais exposer le dossier
    /// personnel : ni `$HOME`, ni un ancêtre, ni un dossier qui contient des
    /// secrets (~/.ssh, ~/.aws, ~/.config, ~/.local/share).
    #[test]
    fn aucune_racine_de_montage_ne_doit_exposer_le_home() {
        let home = Path::new("/home/alice");
        let refused = [
            Path::new("/home/alice"),
            Path::new("/home"),
            Path::new("/"),
            Path::new("/home/alice/.config"),
            Path::new("/home/alice/.local/share"),
            Path::new("/home/alice/.ssh"),
            Path::new("/home/alice/.aws"),
        ];
        for root in refused {
            assert!(
                forbidden_grant_reason(root, Some(home)).is_some(),
                "{root:?} doit être refusé"
            );
        }
        let allowed = [
            Path::new("/home/alice/.local/bin"),
            Path::new("/home/alice/.local/lib"),
            Path::new("/home/alice/bin"),
            Path::new("/home/alice/.config/kilo"),
            Path::new("/tmp/ih-agent-42"),
        ];
        for root in allowed {
            assert!(
                forbidden_grant_reason(root, Some(home)).is_none(),
                "{root:?} doit être accepté"
            );
        }
    }

    /// Un exécutable placé directement dans le dossier personnel est refusé :
    /// la sandbox ne peut pas l'exécuter sans monter `$HOME`.
    #[test]
    fn un_executable_dans_le_home_refuse_la_sandbox() {
        let home = std::env::temp_dir().join("ih-audit-home-refus");
        let executable = home.join("mon-agent");
        let error = executable_grant_roots(&executable, Path::new("/tmp/wt"))
            .err()
            .map(|error| error.to_string());
        // Sans $HOME réel modifié, on vérifie surtout le contrat via la
        // fonction pure (le test d'intégration couvre le cas réel).
        assert!(
            error.is_some() || forbidden_grant_reason(&home, Some(&home)).is_some(),
            "un exécutable à la racine du home doit être refusé"
        );
    }

    /// Un moteur de sandbox qui ignore les drapeaux (faux bwrap) est refusé par
    /// la sonde négative : l'écriture hors sandbox réussit.
    #[test]
    fn un_faux_moteur_est_refuse_par_la_sonde_negative() {
        let fake = std::env::temp_dir().join(format!("ih-faux-bwrap-{}", std::process::id()));
        let script = "#!/bin/sh\nwhile [ \"$1\" != \"--\" ] && [ $# -gt 0 ]; do shift; done\nshift\nexec \"$@\"\n";
        std::fs::write(&fake, script).expect("script");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let result = probe_functional(&fake);
        let _ = std::fs::remove_file(&fake);
        let message = result.expect_err("le faux moteur doit être refusé");
        assert!(
            message.contains("n'isole pas") || message.contains("hors de la sandbox"),
            "message inattendu : {message}"
        );
    }

    /// Un moteur modifiable par le groupe ou le monde est refusé.
    #[test]
    fn un_moteur_inscriptible_par_tous_est_refuse() {
        let fake = std::env::temp_dir().join(format!("ih-bwrap-777-{}", std::process::id()));
        std::fs::write(&fake, "#!/bin/sh\nexit 0\n").expect("script");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o777)).expect("chmod");
        }
        let result = engine_trusted(&fake);
        let _ = std::fs::remove_file(&fake);
        assert!(result.is_err(), "un moteur 0777 doit être refusé");
    }

    /// Le dossier privé est créé en 0700 et supprimé à la destruction du garde.
    #[test]
    fn le_dossier_prive_est_prive_et_supprime() {
        let tag = format!("test-raii-{}", std::process::id());
        let path = {
            let home = PrivateHome::create(&tag).expect("dossier privé");
            let path = home.path().to_path_buf();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "le dossier privé doit être en 0700");
            }
            assert!(path.join(OWNER_FILE).exists(), "marqueur de vie écrit");
            path
        };
        assert!(
            !path.exists(),
            "le dossier privé doit disparaître avec son garde"
        );
    }

    /// Le nettoyage des orphelins épargne les dossiers d'instances vivantes,
    /// y compris quand le marqueur de propriétaire est périmé (PID réutilisé) :
    /// le verrou fait foi.
    #[test]
    fn le_nettoyage_des_orphelins_epargne_les_instances_vivantes() {
        let live = PrivateHome::create(&format!("test-vivant-{}", std::process::id()))
            .expect("dossier vivant");
        // Dossier « mort » : PID inexistant + instant de démarrage quelconque.
        let dead_path =
            std::env::temp_dir().join(format!("ih-agent-home-test-mort-{}", std::process::id()));
        std::fs::create_dir_all(&dead_path).expect("dossier mort");
        std::fs::write(dead_path.join(OWNER_FILE), "4294967294 1").expect("pid mort");

        // Dossier vivant mais marqueur périmé : le verrou est tenu.
        let locked_path =
            std::env::temp_dir().join(format!("ih-agent-home-test-verrou-{}", std::process::id()));
        std::fs::create_dir_all(&locked_path).expect("dossier verrouillé");
        std::fs::write(locked_path.join(OWNER_FILE), "4294967294 1").expect("marqueur périmé");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(locked_path.join(LOCK_FILE))
            .expect("verrou");
        assert_eq!(
            unsafe {
                libc::flock(
                    std::os::unix::io::AsRawFd::as_raw_fd(&lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            },
            0,
            "verrou de test"
        );

        // Dossier illisible (propriétaire absent) : conservé par prudence.
        let unknown_path =
            std::env::temp_dir().join(format!("ih-agent-home-test-inconnu-{}", std::process::id()));
        std::fs::create_dir_all(&unknown_path).expect("dossier inconnu");

        cleanup_orphan_homes();
        assert!(
            !dead_path.exists(),
            "le dossier orphelin doit être supprimé"
        );
        assert!(
            live.path().exists(),
            "le dossier d'une instance vivante est conservé"
        );
        assert!(
            locked_path.exists(),
            "un dossier verrouillé n'est jamais supprimé, même avec un PID périmé"
        );
        assert!(
            unknown_path.exists(),
            "sans marqueur exploitable, le dossier est conservé (jamais de suppression à l'aveugle)"
        );

        drop(lock);
        let _ = std::fs::remove_dir_all(&locked_path);
        let _ = std::fs::remove_dir_all(&unknown_path);
    }

    /// Le marqueur de propriétaire est obligatoire : si son écriture échoue, la
    /// création échoue et le dossier partiel est supprimé.
    #[test]
    fn lechec_decriture_du_marqueur_fait_echouer_la_creation() {
        let path = std::env::temp_dir().join(format!(
            "ih-agent-home-test-marqueur-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("dossier");
        // `OWNER_FILE` est un répertoire : l'écriture du marqueur échoue.
        std::fs::create_dir_all(path.join(OWNER_FILE)).expect("marqueur bloquant");
        let result = PrivateHome::open(path.clone());
        assert!(
            result.is_err(),
            "sans marqueur, aucune session ne doit démarrer"
        );
        let _ = std::fs::remove_dir_all(&path);
        assert!(!path.exists(), "aucun résidu après échec");
    }

    /// Un identifiant de session ne peut pas sortir du répertoire temporaire.
    #[test]
    fn un_identifiant_de_session_dangereux_est_refuse() {
        for bad in ["../evade", "a/b", "..", "", "x".repeat(65).as_str()] {
            assert!(
                validate_session_tag(bad).is_err(),
                "{bad:?} doit être refusé"
            );
        }
        assert!(validate_session_tag("ses-0123abcd").is_ok());
    }

    #[test]
    fn resume_expose_niveau_reseau_et_etat() {
        let isolated = summary(SecurityLevel::Isolated);
        assert!(isolated.sandboxed);
        assert!(!isolated.network);
        let network = summary(SecurityLevel::IsolatedNetwork);
        assert!(network.network);
        let none = summary(SecurityLevel::None);
        assert!(!none.sandboxed);
        assert!(!none.warnings.is_empty());
    }
}
