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

/// Détecte Bubblewrap et vérifie qu'il peut créer une sandbox minimale.
///
/// `IH_SANDBOX_ENGINE` force le chemin du moteur : les tests utilisent ce point
/// d'entrée pour simuler une machine **sans** sandbox (CI GitHub, où bubblewrap
/// absent ou espace de noms non privilégié interdit) sans dépendre du système.
pub fn availability() -> SandboxAvailability {
    if let Some(forced) = std::env::var_os("IH_SANDBOX_ENGINE") {
        let path = PathBuf::from(forced);
        let found = path.is_file();
        let version = if found { probe_version(&path) } else { None };
        let probe = found.then(|| probe_functional(&path));
        return SandboxAvailability {
            engine: "bubblewrap".into(),
            path: found.then(|| path.display().to_string()),
            version,
            functional: matches!(probe, Some(Ok(()))),
            error: match probe {
                Some(Err(message)) => Some(message),
                None => Some(format!(
                    "moteur de sandbox forcé introuvable : {}",
                    path.display()
                )),
                _ => None,
            },
        };
    }
    let Some(path) = ih_core::platform::find_in_path("bwrap").or_else(|| {
        Path::new("/usr/bin/bwrap")
            .exists()
            .then(|| PathBuf::from("/usr/bin/bwrap"))
    }) else {
        return SandboxAvailability {
            engine: "bubblewrap".into(),
            path: None,
            version: None,
            functional: false,
            error: Some(
                "bubblewrap (bwrap) introuvable ; installez-le (paquet « bubblewrap »)".into(),
            ),
        };
    };
    let version = probe_version(&path);
    let probe = probe_functional(&path);
    SandboxAvailability {
        engine: "bubblewrap".into(),
        path: Some(path.display().to_string()),
        version,
        functional: probe.is_ok(),
        error: probe.err(),
    }
}

/// Sonde réelle : démarre une sandbox minimale. Un moteur qui échoue (espace de
/// noms non privilégié interdit, AppArmor, etc.) est **non fonctionnel**.
fn probe_functional(path: &Path) -> Result<(), String> {
    let probe = Command::new(path)
        .args([
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
            "--",
            "/bin/sh",
            "-c",
            "exit 0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("bubblewrap inutilisable : {error}"))?;
    if probe.status.success() {
        Ok(())
    } else {
        Err(format!(
            "bubblewrap refuse de démarrer (user namespaces restreints ?) : {}",
            String::from_utf8_lossy(&probe.stderr).trim()
        ))
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

/// Commande prête à être lancée (programme + arguments) et dossier privé à
/// nettoyer après la session.
pub struct SandboxCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Variables à fournir explicitement (l'environnement hôte n'est pas hérité).
    pub env: Vec<(String, String)>,
    pub private_home: Option<PathBuf>,
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
    report
        .path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| IhError::Agent("chemin de bubblewrap introuvable".into()))
}

/// Construit la commande sandboxée pour un agent.
///
/// `workdir` est monté en lecture-écriture ; tout le reste du système visible
/// est en lecture seule (`/usr`, `/etc`, `/opt/apps`), `/tmp` est privé, le
/// dossier personnel de l'utilisateur n'est jamais monté.
pub fn sandboxed_command(
    level: SecurityLevel,
    kind: &AgentKind,
    executable: &Path,
    args: &[String],
    workdir: &Path,
    extra_env: &[(String, String)],
    session_tag: &str,
) -> IhResult<SandboxCommand> {
    if level == SecurityLevel::None {
        return Err(IhError::Invalid(
            "niveau non isolé : utiliser la commande directe, après confirmation".into(),
        ));
    }
    let bwrap = check_engine(&availability())?;

    let private_home = std::env::temp_dir().join(format!("ih-agent-home-{session_tag}"));
    if private_home.exists() {
        let _ = std::fs::remove_dir_all(&private_home);
    }
    std::fs::create_dir_all(&private_home)
        .map_err(|error| IhError::Internal(format!("dossier privé de l'agent : {error}")))?;

    // Le binaire système est lié tel quel ; un binaire utilisateur est monté
    // en lecture seule. La cible est d'abord RÉSOLUE : beaucoup de CLI sont des
    // liens symboliques (`dsh -> ../lib/node_modules/@deepseek-ai/dsh/lib/bin.js`)
    // et le lien seul ne suffirait pas dans la sandbox.
    let resolved_executable = std::fs::canonicalize(executable)
        .ok()
        .filter(|path| path.exists())
        .unwrap_or_else(|| executable.to_path_buf());
    let mut executable_grants: Vec<(PathBuf, String)> = Vec::new();
    if resolved_executable.starts_with("/opt/ih-agents/bin") {
        // déjà fourni par les montages de la famille (Kilo/Goose)
    } else if !resolved_executable.starts_with("/usr")
        && !resolved_executable.starts_with("/bin")
        && !resolved_executable.starts_with("/opt/apps")
    {
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
        if let Some(root) = root {
            // Un exécutable situé dans le worktree est déjà accessible en
            // lecture-écriture : inutile de le remonter, et un montage en
            // lecture seule masquerait le worktree inscriptible.
            // Un dossier ANCÊTRE du worktree (ex. /tmp/ih-team-123) est monté
            // en lecture seule AVANT le worktree : le montage inscriptible du
            // worktree est appliqué ensuite, par-dessus, et reste gagnant.
            if !root.starts_with(workdir) {
                executable_grants.push((root.clone(), root.display().to_string()));
            }
        }
    }

    let staging = staging_for(kind, &private_home);

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
        private_home.display().to_string(),
        "/home/agent".into(),
    ]);

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

/// Supprime le dossier personnel privé d'une session (secrets copiés compris).
pub fn cleanup_private_home(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
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
        )
        .is_err());
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
