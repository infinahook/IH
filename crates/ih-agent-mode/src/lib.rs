//! Mode Agent d'IH : isolation par worktree, détection d'adaptateurs et
//! collecte des modifications d'une session.
//!
//! # Principe d'isolation
//!
//! Un CLI d'agent modifie les fichiers qu'il veut, sans protocole
//! d'approbation standard. IH ne prétend donc pas intercepter chaque écriture :
//! il exécute l'agent dans un **worktree git détaché** (`git worktree add
//! --detach`). Le projet de l'utilisateur n'est pas modifié pendant la session.
//! À la fin, IH calcule les changements entre le worktree et le projet, et
//! l'application de ces changements passe par le système de diff de la 0.2.1
//! (acceptation explicite, contrôle d'empreinte, sauvegarde).
//!
//! # Limites assumées
//!
//! - Projet **git** requis : hors dépôt git, aucune session n'est lancée.
//! - L'isolation est celle du système de fichiers : un agent qui écrit hors de
//!   son répertoire de travail (chemin absolu, `/tmp`, réseau) n'est pas
//!   empêché. Le CLI conserve ses propres garde-fous.
//! - Aucun secret n'est injecté dans l'environnement de l'agent.

use ih_agents::AgentAdapter;
use ih_core::config::{AgentConfig, AgentKind};
use ih_core::{IhError, IhResult};
use ih_workspace::diff::diff_lines;
use ih_workspace::git::GitRepo;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Taille maximale d'un fichier lu pour préparer un diff.
pub const MAX_CHANGE_BYTES: usize = 512 * 1024;
/// Nombre maximal de fichiers suivis par session.
pub const MAX_CHANGED_FILES: usize = 200;
/// Délai maximal d'exécution de `--version`.
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------- adaptateurs

/// État d'un adaptateur, exposé à l'interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterReport {
    pub id: String,
    pub label: String,
    pub kind: AgentKind,
    pub program: String,
    pub detected: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// Vrai si l'exécutable existe ET répond correctement à `--version` : un
    /// binaire homonyme non fonctionnel ne doit pas être proposé au lancement.
    pub functional: bool,
    /// Niveau de sécurité configuré et état réel de la sandbox.
    pub security: ih_core::config::SecurityLevel,
    pub sandbox: ih_agents::sandbox::SandboxSummary,
    /// Capacités réellement garanties par le contrat d'IH.
    pub capabilities: Vec<String>,
    pub diagnostic: String,
}

/// Détecte les adaptateurs configurés et tente de lire leur version.
pub fn detect_adapters(configs: &[AgentConfig]) -> Vec<AdapterReport> {
    configs
        .iter()
        .map(|config| {
            let adapter = AgentAdapter::from_config(config);
            let autonomy = adapter.autonomy_flags();
            let path = adapter.detection();
            // Un adaptateur personnalisé peut être un script : exécuter
            // `--version` y produirait des effets de bord. On ne sonde que les
            // CLI connus, toujours dans un répertoire neutre.
            let version = if config.kind == AgentKind::Custom {
                None
            } else {
                path.as_deref().and_then(probe_version)
            };
            let detected = path.is_some();
            let functional = detected && (config.kind == AgentKind::Custom || version.is_some());
            AdapterReport {
                id: config.id.clone(),
                label: config.label.clone(),
                kind: config.kind.clone(),
                program: config.program.clone(),
                detected,
                path: path.map(|p| p.display().to_string()),
                version,
                functional,
                security: config.security,
                sandbox: ih_agents::sandbox::summary(config.security),
                capabilities: {
                    let mut caps = capabilities_for(&config.kind);
                    if !autonomy.is_empty() {
                        caps.push(format!("outils-autonomes ({})", autonomy.join(" ")));
                    }
                    caps
                },
                diagnostic: if functional {
                    "exécutable trouvé et fonctionnel".into()
                } else if detected {
                    format!(
                        "« {} » trouvé mais ne répond pas à --version : CLI attendu ?",
                        config.program
                    )
                } else {
                    format!(
                        "« {} » introuvable (PATH, PATH du shell de connexion, dossiers connus)",
                        config.program
                    )
                },
            }
        })
        .collect()
}

/// Capacités garanties par le contrat d'IH pour chaque famille d'adaptateur.
fn capabilities_for(kind: &AgentKind) -> Vec<String> {
    let mut caps = vec![
        "prompt-non-interactif".to_string(),
        "sortie-flux".to_string(),
        "repertoire-travail".to_string(),
        "annulation".to_string(),
        "timeout".to_string(),
    ];
    match kind {
        // Interfaces structurées officielles : événements normalisés par IH.
        AgentKind::Opencode | AgentKind::Kilo => caps.push("sortie-json".to_string()),
        AgentKind::Goose => caps.push("sortie-json".to_string()),
        // DSH headless n'imprime que le message final : sortie texte.
        AgentKind::Dsh => caps.push("sortie-texte-finale".to_string()),
        AgentKind::ClaudeCode
        | AgentKind::Codex
        | AgentKind::GeminiCli
        | AgentKind::Pi
        | AgentKind::Custom => {}
    }
    if matches!(kind, AgentKind::Custom) {
        caps.push("arguments-personnalises".to_string());
    }
    caps
}

/// Lit la première ligne de `<program> --version`, bornée et sous délai.
pub fn probe_version(program: &Path) -> Option<String> {
    // Répertoire neutre : un CLI qui écrirait pendant `--version` ne doit pas
    // toucher au projet ni au répertoire courant d'IH.
    let neutral = std::env::temp_dir().join("ih-version-probe");
    // Répertoire neutre garanti : si sa création échoue (TMPDIR exotique en CI),
    // on retombe sur le répertoire temporaire lui-même, puis sur le parent du
    // programme — un `current_dir` invalide ferait échouer le spawn.
    if std::fs::create_dir_all(&neutral).is_err() || !neutral.is_dir() {
        let fallback = std::env::temp_dir();
        if fallback.is_dir() {
            std::fs::create_dir_all(&neutral).ok();
        }
    }
    let neutral = if neutral.is_dir() {
        neutral
    } else {
        program
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf()
    };
    // `ETXTBSY` peut survenir transitoirement quand un autre test (ou un
    // `fork` concurrent) détient encore le descripteur d'écriture du script :
    // la sonde réessaie quelques instants au lieu d'abandonner la version.
    let mut child = {
        let mut spawned = None;
        for attempt in 0..20 {
            match Command::new(program)
                .arg("--version")
                .current_dir(&neutral)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    spawned = Some(child);
                    break;
                }
                Err(error) if attempt < 19 && error.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        spawned?
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) if started.elapsed() > VERSION_TIMEOUT => {
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
    Some(line.chars().take(120).collect())
}

// ------------------------------------------------------------------- sandbox

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sandbox {
    project_root: PathBuf,
    workdir: PathBuf,
    /// Chemins Git attendus, calculés ici à partir du dépôt découvert par IH.
    /// Le fichier `.git` du worktree n'est jamais consulté comme autorité.
    git: GitMounts,
}

impl Sandbox {
    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Montages Git attendus (à transmettre au superviseur).
    pub fn git(&self) -> &GitMounts {
        &self.git
    }
}

/// Crée un worktree détaché isolé pour une session.
///
/// `base_dir` est créé si nécessaire ; le worktree est placé dans
/// `<base_dir>/<tag>`. Le projet doit être un dépôt git.
pub fn create_sandbox(project_root: &Path, base_dir: &Path, tag: &str) -> IhResult<Sandbox> {
    if tag.trim().is_empty() || tag.contains('/') || tag.contains("..") {
        return Err(IhError::Invalid(format!(
            "étiquette de session invalide : {tag}"
        )));
    }
    let repo = GitRepo::discover(project_root).map_err(|_| {
        IhError::Unsupported(
            "mode Agent : le projet doit être un dépôt git (isolation par worktree)".into(),
        )
    })?;
    std::fs::create_dir_all(base_dir)?;
    let workdir = base_dir.join(tag);
    if workdir.exists() {
        return Err(IhError::Conflict(format!(
            "le répertoire de session existe déjà : {}",
            workdir.display()
        )));
    }
    repo.add_detached_worktree(&workdir)?;
    // Le dépôt `.git` du projet et le nom du worktree sont connus d'IH : les
    // montages sont calculés ici, puis vérifiés (canonicalisation + confinement
    // + pointeur `.git` conforme). Un `.git` modifié fait échouer la création.
    let common_dir = repo.git_common_dir()?;
    let name = workdir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| IhError::Invalid("nom de worktree non UTF-8".into()))?;
    let git = match GitMounts::expected(&common_dir, name, &workdir) {
        Ok(git) => git,
        Err(error) => {
            // Worktree inutilisable : on le retire au lieu de laisser une
            // session à moitié créée.
            let _ = repo.remove_worktree(&workdir, true);
            return Err(error);
        }
    };
    Ok(Sandbox {
        project_root: repo.root().to_path_buf(),
        workdir,
        git,
    })
}

/// Supprime le worktree et le répertoire de session.
pub fn remove_sandbox(sandbox: &Sandbox) -> IhResult<()> {
    if let Ok(repo) = GitRepo::discover(&sandbox.project_root) {
        let _ = repo.remove_worktree(&sandbox.workdir, true);
    }
    if sandbox.workdir.exists() {
        std::fs::remove_dir_all(&sandbox.workdir)?;
    }
    Ok(())
}

/// Commande Git **durcie** pour inspecter un worktree contrôlé par un agent.
///
/// Le dépôt peut contenir des configurations ou attributs exécutables
/// (`core.fsmonitor`, `core.hooksPath`, `diff.<driver>.textconv`, `diff.external`,
/// protocoles externes) : ils sont neutralisés par configuration explicite en
/// ligne de commande (prioritaire sur toute configuration de dépôt) et par un
/// environnement minimal. Un agent ne peut donc pas faire exécuter un programme
/// sur l'hôte en modifiant des fichiers Git.
fn hardened_git(workdir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", std::env::temp_dir())
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false")
        .arg("-C")
        .arg(workdir)
        .args([
            // Neutralisation des mécanismes exécutables du dépôt.
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.pager=cat",
            "-c",
            "core.askPass=",
            "-c",
            "core.sshCommand=",
            "-c",
            "core.editor=/bin/false",
            "-c",
            "diff.external=",
            "-c",
            "diff.renames=false",
            "-c",
            "protocol.allow=never",
            "-c",
            "submodule.recurse=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
        ]);
    command
}

/// Liste les fichiers modifiés, ajoutés ou supprimés dans le sandbox.
pub fn changed_files(sandbox: &Sandbox) -> IhResult<Vec<String>> {
    let output = hardened_git(&sandbox.workdir)
        .args(["status", "--porcelain=v1", "-z"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| IhError::Internal(format!("git status indisponible : {e}")))?;
    if !output.status.success() {
        return Err(IhError::Internal(format!(
            "git status a échoué : {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    let mut entries = raw.split('\0').filter(|s| !s.is_empty()).peekable();
    let mut files = Vec::new();
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let status = &entry[..2];
        let path = entry[3..].to_string();
        // Renommage : l'ancien chemin suit dans l'entrée suivante.
        if status.starts_with('R') || status.starts_with('C') {
            let _ancien = entries.next();
        }
        if !files.contains(&path) {
            files.push(path);
        }
        if files.len() >= MAX_CHANGED_FILES {
            break;
        }
    }
    Ok(files)
}

/// Modification d'un fichier entre le projet et le sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// Contenu dans le projet (`None` si le fichier est nouveau).
    pub before: Option<String>,
    /// Contenu produit par l'agent (`None` si le fichier a été supprimé).
    pub after: Option<String>,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub too_large: bool,
}

/// Construit les changements fichier par fichier, bornés en taille.
pub fn collect_changes(sandbox: &Sandbox, max_bytes: usize) -> IhResult<Vec<FileChange>> {
    let limit = max_bytes.clamp(1024, MAX_CHANGE_BYTES);
    let mut changes = Vec::new();
    for path in changed_files(sandbox)? {
        if path.contains("..") || Path::new(&path).is_absolute() {
            continue;
        }
        let before_path = sandbox.project_root.join(&path);
        let after_path = sandbox.workdir.join(&path);

        let (before, before_binary, before_large) = read_optional(&before_path, limit)?;
        let (after, after_binary, after_large) = read_optional(&after_path, limit)?;
        let binary = before_binary || after_binary;
        let too_large = before_large || after_large;

        let (additions, deletions) = if binary || too_large {
            (0, 0)
        } else {
            let diff = diff_lines(
                before.as_deref().unwrap_or(""),
                after.as_deref().unwrap_or(""),
            );
            (diff.additions, diff.deletions)
        };

        changes.push(FileChange {
            path,
            before,
            after,
            additions,
            deletions,
            binary,
            too_large,
        });
    }
    Ok(changes)
}

fn read_optional(path: &Path, limit: usize) -> IhResult<(Option<String>, bool, bool)> {
    if !path.exists() {
        return Ok((None, false, false));
    }
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Ok((None, false, false));
    }
    if meta.len() as usize > limit {
        return Ok((None, false, true));
    }
    let bytes = std::fs::read(path)?;
    if ih_workspace::fs::looks_binary(&bytes) {
        return Ok((None, true, false));
    }
    Ok((
        Some(String::from_utf8_lossy(&bytes).into_owned()),
        false,
        false,
    ))
}

// ---------------------------------------------------------------- approbation

/// Empreinte de l'action autorisée : toute modification de la commande, du
/// répertoire ou de la tâche change l'empreinte et invalide l'approbation.
pub fn action_fingerprint(agent_id: &str, command: &str, workdir: &Path, prompt: &str) -> String {
    let payload = format!(
        "agent={agent_id}\ncommand={command}\nworkdir={}\nprompt={prompt}",
        workdir.display()
    );
    ih_workspace::fs::sha256_hex(payload.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ih-am-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?} : {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn git_project(tag: &str) -> PathBuf {
        let dir = temp_dir(tag);
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "test@ih"]);
        git(&dir, &["config", "user.name", "IH Test"]);
        std::fs::write(dir.join("README.md"), "# projet\n").unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "initial"]);
        dir
    }

    fn fake_agent(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        let staging = dir.join(format!(".{name}.tmp"));
        std::fs::write(&staging, body).unwrap();
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&staging, &path).unwrap();
        path
    }

    #[test]
    fn sandbox_isole_du_projet() {
        let project = git_project("sandbox");
        let base = temp_dir("wt");
        let sandbox = create_sandbox(&project, &base, "sess-1").unwrap();
        assert!(sandbox.workdir().is_dir());
        assert_eq!(changed_files(&sandbox).unwrap(), Vec::<String>::new());

        std::fs::write(
            sandbox.workdir().join("src/main.rs"),
            "fn main() { /* agent */ }\n",
        )
        .unwrap();
        let changes = changed_files(&sandbox).unwrap();
        assert_eq!(changes, vec!["src/main.rs".to_string()]);

        // Le projet n'a pas bougé.
        assert_eq!(
            std::fs::read_to_string(project.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        remove_sandbox(&sandbox).unwrap();
        assert!(!sandbox.workdir().exists());
        std::fs::remove_dir_all(&project).ok();
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn collecte_ajout_modification_suppression() {
        let project = git_project("collecte");
        let base = temp_dir("wt2");
        let sandbox = create_sandbox(&project, &base, "sess-2").unwrap();

        // modification
        std::fs::write(
            sandbox.workdir().join("src/main.rs"),
            "fn main() {\n    println!(\"x\");\n}\n",
        )
        .unwrap();
        // ajout
        std::fs::write(sandbox.workdir().join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        // suppression
        std::fs::remove_file(sandbox.workdir().join("README.md")).unwrap();

        let changes = collect_changes(&sandbox, MAX_CHANGE_BYTES).unwrap();
        let main = changes.iter().find(|c| c.path == "src/main.rs").unwrap();
        assert!(main.before.is_some() && main.after.is_some());
        assert!(main.additions >= 1 && main.deletions >= 1);

        let lib = changes.iter().find(|c| c.path == "src/lib.rs").unwrap();
        assert!(lib.before.is_none(), "nouveau fichier");
        assert_eq!(lib.after.as_deref(), Some("pub fn f() {}\n"));

        let readme = changes.iter().find(|c| c.path == "README.md").unwrap();
        assert!(readme.after.is_none(), "fichier supprimé");

        remove_sandbox(&sandbox).unwrap();
        std::fs::remove_dir_all(&project).ok();
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn hors_depot_git_refuse() {
        let dir = temp_dir("nogit");
        let base = temp_dir("wt3");
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let error = create_sandbox(&dir, &base, "s1").unwrap_err();
        assert!(matches!(error, IhError::Unsupported(_)));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn les_drapeaux_d_autonomie_sont_exposes_dans_les_capacites() {
        // Script factice identifié comme Kilo : la sonde de version réussit et
        // le rapport expose l'autonomie apportée par `--auto`.
        let dir = std::env::temp_dir().join(format!("ih-mode-autonomy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("kilo");
        std::fs::write(&script, "#!/bin/sh\necho 7.8.32\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let configs = vec![AgentConfig {
            id: "kilo".into(),
            kind: AgentKind::Kilo,
            label: "Kilo".into(),
            program: script.display().to_string(),
            enabled: true,
            ..Default::default()
        }];
        let reports = detect_adapters(&configs);
        assert!(
            reports[0]
                .capabilities
                .iter()
                .any(|capability| capability == "outils-autonomes (--auto)"),
            "capacités : {:?}",
            reports[0].capabilities
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detection_et_version_des_adaptateurs() {
        let dir = temp_dir("adapter");
        let script = fake_agent(&dir, "fake-agent", "#!/bin/sh\necho \"fake-agent 1.2.3\"\n");
        let configs = vec![
            // CLI connu : la version est sondée (dans un répertoire neutre).
            AgentConfig {
                id: "fake".into(),
                kind: AgentKind::ClaudeCode,
                label: "Faux".into(),
                program: script.display().to_string(),
                enabled: true,
                ..Default::default()
            },
            // Adaptateur personnalisé : jamais exécuté pour `--version`.
            AgentConfig {
                id: "perso".into(),
                kind: AgentKind::Custom,
                label: "Perso".into(),
                program: script.display().to_string(),
                enabled: true,
                ..Default::default()
            },
            AgentConfig {
                id: "absent".into(),
                kind: AgentKind::Custom,
                label: "Absent".into(),
                program: "agent-inexistant-ih".into(),
                enabled: true,
                ..Default::default()
            },
        ];
        let reports = detect_adapters(&configs);
        assert!(reports[0].detected);
        assert_eq!(
            reports[0].version.as_deref(),
            Some("fake-agent 1.2.3"),
            "diagnostic={:?} chemin={:?} exécutable={:?}",
            reports[0].diagnostic,
            reports[0].path,
            std::fs::metadata(&script).map(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o111
            })
        );
        assert!(reports[0].capabilities.contains(&"annulation".to_string()));
        assert!(reports[1].detected);
        assert!(
            reports[1].version.is_none(),
            "un adaptateur personnalisé ne doit pas être exécuté pour --version"
        );
        assert!(!reports[2].detected);
        assert!(reports[2].version.is_none());
        assert!(reports[2].diagnostic.contains("introuvable"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empreinte_change_avec_la_commande_ou_la_tache() {
        let dir = PathBuf::from("/tmp/ih-action");
        let a = action_fingerprint("claude-code", "claude -p x", &dir, "tâche");
        let b = action_fingerprint("claude-code", "claude -p y", &dir, "tâche");
        let c = action_fingerprint("claude-code", "claude -p x", &dir, "autre tâche");
        let d = action_fingerprint(
            "claude-code",
            "claude -p x",
            &PathBuf::from("/tmp/autre"),
            "tâche",
        );
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a.len(), 64);
        assert_eq!(
            a,
            action_fingerprint("claude-code", "claude -p x", &dir, "tâche")
        );
    }
}

// ---------------------------------------------------------------------------
// Exécution d'une équipe : une session d'agent réelle par tâche, dans un
// worktree unique. Extrait du commanditaire Tauri pour rester testable.
// ---------------------------------------------------------------------------

use ih_agents::sandbox::GitMounts;
use ih_agents::{AgentSupervisor, StartRequest};
use ih_core::config::AppConfig;
use ih_core::events::AgentState;
use ih_orchestrator::Plan;

/// Résultat d'une tâche d'équipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamTaskOutcome {
    pub task_id: String,
    pub title: String,
    pub role: String,
    pub agent_id: String,
    pub state: String,
}

/// Exécute les tâches du plan, dans l'ordre fourni, dans `sandbox`.
///
/// S'arrête à la première erreur ; les tâches suivantes sont marquées
/// `cancelled`. Chaque tâche lance **sa propre session** (aucun mock).
pub fn run_team_tasks(
    supervisor: &AgentSupervisor,
    sandbox: &Sandbox,
    config: &AppConfig,
    plan: &Plan,
    goal: &str,
    task_timeout: Duration,
) -> IhResult<Vec<TeamTaskOutcome>> {
    run_team_tasks_with_events(
        supervisor,
        sandbox,
        config,
        plan,
        goal,
        task_timeout,
        &mut |_| {},
    )
}

/// Comme [`run_team_tasks`], en notifiant chaque tâche (démarrage et fin).
pub fn run_team_tasks_with_events(
    supervisor: &AgentSupervisor,
    sandbox: &Sandbox,
    config: &AppConfig,
    plan: &Plan,
    goal: &str,
    task_timeout: Duration,
    on_task: &mut dyn FnMut(&TeamTaskOutcome),
) -> IhResult<Vec<TeamTaskOutcome>> {
    let mut outcomes = Vec::new();
    let mut aborted = false;
    // `iter()` suit les identifiants : l'ordre d'exécution vient du tri
    // topologique (lead, puis workers, puis reviewer).
    for node in plan.graph.topological_order() {
        let started = TeamTaskOutcome {
            task_id: node.id.to_string(),
            title: node.title.clone(),
            role: node.role.as_str().to_string(),
            agent_id: node.assignee.clone().unwrap_or_default(),
            state: "running".into(),
        };
        on_task(&started);

        if aborted {
            let cancelled = TeamTaskOutcome {
                task_id: node.id.to_string(),
                title: node.title.clone(),
                role: node.role.as_str().to_string(),
                agent_id: node.assignee.clone().unwrap_or_default(),
                state: "cancelled".into(),
            };
            on_task(&cancelled);
            outcomes.push(TeamTaskOutcome {
                task_id: node.id.to_string(),
                title: node.title.clone(),
                role: node.role.as_str().to_string(),
                agent_id: node.assignee.clone().unwrap_or_default(),
                state: "cancelled".into(),
            });
            continue;
        }
        let agent_id = node
            .assignee
            .clone()
            .ok_or_else(|| IhError::Invalid(format!("tâche sans agent : {}", node.title)))?;
        let agent = config
            .agent(&agent_id)
            .ok_or_else(|| IhError::NotFound(format!("agent {agent_id}")))?;
        let adapter = AgentAdapter::from_config(agent);
        if adapter.detection().is_none() {
            aborted = true;
            outcomes.push(TeamTaskOutcome {
                task_id: node.id.to_string(),
                title: node.title.clone(),
                role: node.role.as_str().to_string(),
                agent_id,
                state: "failed".into(),
            });
            continue;
        }
        let prompt = format!(
            "Rôle : {}\nObjectif global : {}\nTâche : {}\n\n{}",
            node.role.as_str(),
            goal,
            node.title,
            node.description
        );
        let unsandboxed = adapter.security == ih_core::config::SecurityLevel::None;
        let state = match supervisor.start(StartRequest {
            agent: adapter,
            project_dir: sandbox.workdir().to_path_buf(),
            prompt,
            session_id: None,
            git: Some(sandbox.git().clone()),
            // Confirmation : uniquement pour un membre réellement configuré au
            // niveau non isolé, et seulement parce que la course d'équipe a été
            // approuvée (acquittement explicite inclus dans l'empreinte).
            confirm_unsandboxed: unsandboxed,
        }) {
            Ok(session) => supervisor
                .wait(&session, task_timeout)
                .unwrap_or(AgentState::Failed),
            Err(_) => AgentState::Failed,
        };
        let state = match state {
            AgentState::Completed => "completed",
            AgentState::Cancelled => "cancelled",
            _ => "failed",
        };
        if state != "completed" {
            aborted = true;
        }
        let finished = TeamTaskOutcome {
            task_id: node.id.to_string(),
            title: node.title.clone(),
            role: node.role.as_str().to_string(),
            agent_id,
            state: state.into(),
        };
        on_task(&finished);
        outcomes.push(finished);
    }
    Ok(outcomes)
}

// ---------------------------------------------------------------------------
// Empreinte d'approbation d'une course d'équipe.
// ---------------------------------------------------------------------------

/// Une course d'équipe contenant au moins un agent au niveau `none` exige un
/// acquittement explicite de l'utilisateur, comme en Mode Agent.
pub fn team_requires_unsandboxed_ack(
    levels: impl IntoIterator<Item = ih_core::config::SecurityLevel>,
) -> bool {
    levels
        .into_iter()
        .any(|level| level == ih_core::config::SecurityLevel::None)
}

/// Empreinte couvrant **tous** les paramètres qui changent l'exécution :
/// objectif, lead, workers (ordre inclus), relecteur et répertoire de travail.
pub fn team_fingerprint(
    goal: &str,
    lead: &str,
    workers: &[String],
    reviewer: Option<&str>,
    project_root: &std::path::Path,
) -> String {
    let agents = format!(
        "lead={lead}|workers={}|reviewer={}",
        workers.join(","),
        reviewer.unwrap_or("")
    );
    action_fingerprint("team", &agents, project_root, goal)
}

/// Vrai si le détail d'approbation contient exactement cette empreinte.
pub fn approval_covers(detail: &str, fingerprint: &str) -> bool {
    !fingerprint.is_empty() && detail.contains(fingerprint)
}

#[cfg(test)]
mod team_fingerprint_tests {
    use super::{approval_covers, team_fingerprint};
    use std::path::Path;

    fn fp(goal: &str, lead: &str, workers: &[&str], reviewer: Option<&str>, root: &str) -> String {
        let workers: Vec<String> = workers.iter().map(|w| w.to_string()).collect();
        team_fingerprint(goal, lead, &workers, reviewer, Path::new(root))
    }

    #[test]
    fn toute_modification_change_lempreinte() {
        let base = fp("objectif", "opencode", &["pi"], Some("pi"), "/tmp/projet");
        assert_ne!(
            base,
            fp(
                "objectif modifié",
                "opencode",
                &["pi"],
                Some("pi"),
                "/tmp/projet"
            )
        );
        assert_ne!(
            base,
            fp("objectif", "claude", &["pi"], Some("pi"), "/tmp/projet")
        );
        assert_ne!(
            base,
            fp(
                "objectif",
                "opencode",
                &["codex"],
                Some("pi"),
                "/tmp/projet"
            )
        );
        assert_ne!(
            base,
            fp(
                "objectif",
                "opencode",
                &["pi", "codex"],
                Some("pi"),
                "/tmp/projet"
            )
        );
        assert_ne!(
            base,
            fp(
                "objectif",
                "opencode",
                &["pi"],
                Some("opencode"),
                "/tmp/projet"
            )
        );
        assert_ne!(
            base,
            fp("objectif", "opencode", &["pi"], None, "/tmp/projet")
        );
        assert_ne!(
            base,
            fp("objectif", "opencode", &["pi"], Some("pi"), "/tmp/autre")
        );
        // Déterministe à paramètres égaux.
        assert_eq!(
            base,
            fp("objectif", "opencode", &["pi"], Some("pi"), "/tmp/projet")
        );
    }

    #[test]
    fn une_approbation_ne_couvre_que_son_empreinte() {
        let detail = "Objectif : x\nEmpreinte : abc123\n";
        assert!(approval_covers(detail, "abc123"));
        assert!(!approval_covers(detail, "def456"));
        assert!(!approval_covers(detail, ""));
        // Le détail d'une autre course ne couvre pas cette empreinte.
        let other = "Objectif : y\nEmpreinte : zzz999\n";
        assert!(!approval_covers(other, "abc123"));
    }

    /// V3 : l'acquittement explicite est exigé dès qu'un membre de l'équipe
    /// s'exécute sans sandbox, et seulement dans ce cas.
    #[test]
    fn l_acquittement_est_exige_des_qu_un_membre_est_non_isole() {
        use super::team_requires_unsandboxed_ack;
        use ih_core::config::SecurityLevel;
        assert!(!team_requires_unsandboxed_ack([]));
        assert!(!team_requires_unsandboxed_ack([
            SecurityLevel::Isolated,
            SecurityLevel::IsolatedNetwork,
        ]));
        assert!(team_requires_unsandboxed_ack([
            SecurityLevel::IsolatedNetwork,
            SecurityLevel::None,
        ]));
        assert!(team_requires_unsandboxed_ack([SecurityLevel::None]));
    }
}
