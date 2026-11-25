//! Adaptateurs d'agents de développement.
//!
//! Chaque CLI a sa propre façon de recevoir une instruction unique. IH ne
//! devine rien : la commande est construite explicitement et peut être
//! surchargée par la configuration (`args` contenant `{prompt}`).
//!
//! Limite assumée : ces contrats d'appel suivent la documentation publique des
//! CLI. Ils sont testés sur un faux agent, pas contre les binaires réels
//! (Claude Code, Codex, Gemini CLI, OpenCode ne sont pas installés sur la
//! machine de développement). La détection dans le `PATH` et la supervision,
//! elles, sont testées.

use ih_core::config::{AgentConfig, AgentKind};
use ih_core::platform::{find_in_path, login_shell_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Description d'un adaptateur, exposée à l'interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub label: String,
    pub kind: AgentKind,
    pub program: String,
    pub enabled: bool,
    pub detected: bool,
    pub resolved_path: Option<String>,
    /// Version lue par `<exécutable> --version` (jamais inventée).
    pub version: Option<String>,
    /// Diagnostic quand la détection ou la sonde de version échoue.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAdapter {
    pub id: String,
    pub kind: AgentKind,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub enabled: bool,
    pub timeout_secs: u64,
    /// Variables d'environnement déclarées pour cet agent (transmises telles
    /// quelles au processus ; aucun secret n'est ajouté par IH).
    pub env: std::collections::BTreeMap<String, String>,
}

impl AgentAdapter {
    pub fn from_config(config: &AgentConfig) -> Self {
        Self {
            id: config.id.clone(),
            kind: config.kind.clone(),
            label: config.label.clone(),
            program: config.program.clone(),
            args: config.args.clone(),
            enabled: config.enabled,
            timeout_secs: config.timeout_secs,
            env: config.env.clone(),
        }
    }

    pub fn detection(&self) -> Option<PathBuf> {
        resolve_program(&self.program, &self.kind)
    }

    pub fn info(&self) -> AgentInfo {
        let resolved = self.detection();
        // Un script personnalisé peut produire des effets de bord à `--version` :
        // on ne sonde que les CLI connus. Un binaire trouvé mais qui ne répond
        // pas correctement est signalé comme non fonctionnel (faux positif évité).
        let (version, error) = match resolved.as_deref() {
            None => (
                None,
                Some(format!(
                    "« {} » introuvable (PATH, PATH du shell de connexion et dossiers connus)",
                    self.program
                )),
            ),
            Some(_) if self.kind == AgentKind::Custom => (None, None),
            Some(path) => match probe_version(path) {
                Some(version) => (Some(version), None),
                None => (
                    None,
                    Some(format!(
                        "{} ne répond pas à --version : CLI attendu ? (binaire non fonctionnel)",
                        path.display()
                    )),
                ),
            },
        };
        AgentInfo {
            id: self.id.clone(),
            label: self.label.clone(),
            kind: self.kind.clone(),
            program: self.program.clone(),
            enabled: self.enabled,
            detected: resolved.is_some(),
            resolved_path: resolved.map(|p| p.display().to_string()),
            version,
            error,
        }
    }

    /// Drapeaux d'autonomie que la commande réelle contiendra (ex. `--auto`).
    ///
    /// L'approbation IH porte sur la commande exacte : ces drapeaux doivent être
    /// visibles dans le détail d'approbation, car ils élargissent les pouvoirs
    /// de l'agent (outils sans confirmation, écriture/exécution hors worktree).
    pub fn autonomy_flags(&self) -> Vec<String> {
        const KNOWN: [&str; 6] = [
            "--auto",
            "--yolo",
            "--full-auto",
            "--always-approve",
            "--dangerously-skip-permissions",
            "--dangerously-bypass-approvals-and-sandbox",
        ];
        let (_, args) = self.build_command("");
        let mut flags: Vec<String> = args
            .iter()
            .filter(|arg| KNOWN.contains(&arg.as_str()))
            .cloned()
            .collect();
        flags.dedup();
        flags
    }

    /// Construit le couple (programme, arguments) pour une instruction.
    ///
    /// Si `args` contient `{prompt}`, la substitution a lieu sur place ; sinon
    /// l'instruction est ajoutée en fin d'arguments selon le mode non
    /// interactif du CLI.
    pub fn build_command(&self, prompt: &str) -> (String, Vec<String>) {
        if self.args.iter().any(|a| a.contains("{prompt}")) {
            let args = self
                .args
                .iter()
                .map(|a| a.replace("{prompt}", prompt))
                .collect();
            return (self.program.clone(), args);
        }

        let mut args: Vec<String> = self.args.clone();
        match self.kind {
            // `claude -p "<prompt>"` : mode impression, une seule réponse.
            AgentKind::ClaudeCode => {
                args.push("-p".into());
                args.push(prompt.to_string());
            }
            // `codex exec "<prompt>"` : exécution non interactive.
            AgentKind::Codex => {
                if !args.iter().any(|a| a == "exec") {
                    args.push("exec".into());
                }
                args.push(prompt.to_string());
            }
            // `gemini -p "<prompt>"`.
            AgentKind::GeminiCli => {
                args.push("-p".into());
                args.push(prompt.to_string());
            }
            // `opencode run --format json --archived "<prompt>"` : le JSON est
            // normalisé par `normalize` ; `--archived` garde les sessions IH
            // hors de la liste de sessions de l'utilisateur.
            AgentKind::Pi => {
                // Non interactif : traite l'invite et rend la main.
                if !args.iter().any(|a| a == "--print" || a == "-p") {
                    args.push("--print".into());
                }
                if !args.iter().any(|a| a == "--no-session") {
                    args.push("--no-session".into());
                }
                args.push(prompt.to_string());
            }
            AgentKind::Opencode => {
                if !args.iter().any(|a| a == "run") {
                    args.push("run".into());
                }
                if !args.iter().any(|a| a == "--format") {
                    args.push("--format".into());
                    args.push("json".into());
                }
                if !args.iter().any(|a| a == "--archived") {
                    args.push("--archived".into());
                }
                // OBLIGATOIRE : sans serveur privé, OpenCode se connecte au
                // service d'arrière-plan qui résout le projet depuis SON
                // contexte, ignorant le répertoire de travail d'IH (vérifié
                // v2.3.2 : les fichiers sortaient du worktree).
                if !args.iter().any(|a| a == "--standalone") {
                    args.push("--standalone".into());
                }
                args.push(prompt.to_string());
            }
            // `kilo run --format json --auto "<prompt>"` : NDJSON (même
            // famille d'événements qu'OpenCode, vérifié sur kilo 7.8.32).
            //
            // `--auto` est indispensable en non interactif : sans lui, kilo
            // auto-rejette sa toute première demande de permission d'outil et
            // la session échoue (« run ended with an auto-rejected
            // permission »). L'autorisation humaine d'IH (approbation liée à
            // l'empreinte, avant lancement) et l'isolation par worktree
            // restent les seules frontières : rien n'est ajouté sans elles.
            AgentKind::Kilo => {
                if !args.iter().any(|a| a == "run") {
                    args.push("run".into());
                }
                if !args.iter().any(|a| a == "--format") {
                    args.push("--format".into());
                    args.push("json".into());
                }
                if !args.iter().any(|a| a == "--auto") {
                    args.push("--auto".into());
                }
                args.push(prompt.to_string());
            }
            // `dsh --profile headless "<tâche>"` : une tâche, réponse finale,
            // sortie (interface texte officielle du profil headless).
            AgentKind::Dsh => {
                if !args.iter().any(|a| a == "--profile") {
                    args.push("--profile".into());
                    args.push("headless".into());
                }
                args.push(prompt.to_string());
            }
            // `goose run --no-session --quiet --output-format stream-json -t "<prompt>"` :
            // NDJSON officiel (vérifié sur goose 1.61.0), sans bannière.
            AgentKind::Goose => {
                if !args.iter().any(|a| a == "run") {
                    args.push("run".into());
                }
                if !args.iter().any(|a| a == "--output-format") {
                    args.push("--output-format".into());
                    args.push("stream-json".into());
                }
                if !args.iter().any(|a| a == "--no-session") {
                    args.push("--no-session".into());
                }
                if !args.iter().any(|a| a == "--quiet" || a == "-q") {
                    args.push("--quiet".into());
                }
                if !args.iter().any(|a| a == "--text" || a == "-t") {
                    args.push("-t".into());
                    args.push(prompt.to_string());
                }
            }
            AgentKind::Custom => args.push(prompt.to_string()),
        }
        (self.program.clone(), args)
    }
}

// ---------------------------------------------------------------- résolution

/// Noms de binaires acceptés pour une famille d'agent (alias connus).
fn accepted_names(kind: &AgentKind, program: &str) -> Vec<String> {
    let mut names = vec![program.to_string()];
    let aliases: &[&str] = match kind {
        AgentKind::Kilo => &["kilo", "kilocode"],
        AgentKind::Dsh => &["dsh", "dsh-cli"],
        AgentKind::Goose => &["goose"],
        _ => &[],
    };
    for alias in aliases {
        if !names.iter().any(|name| name == alias) {
            names.push((*alias).to_string());
        }
    }
    names
}

/// Dossiers fouillés en plus du PATH : installations utilisateur fréquentes et
/// paquets d'application (Debian/UOS : `/opt/apps/<app>/files/bin`, cas de DSH).
fn extra_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for relative in [
            ".local/bin",
            "bin",
            ".cargo/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".local/share/pnpm",
            ".local/share/uv/tools/bin",
        ] {
            dirs.push(home.join(relative));
        }
    }
    for absolute in [
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/local/sbin",
        "/snap/bin",
        "/var/lib/flatpak/exports/bin",
        "/opt/homebrew/bin",
    ] {
        dirs.push(PathBuf::from(absolute));
    }
    // Paquets d'application : `/opt/apps/*/files/bin`.
    if let Ok(entries) = std::fs::read_dir("/opt/apps") {
        for entry in entries.flatten() {
            let candidate = entry.path().join("files/bin");
            if candidate.is_dir() {
                dirs.push(candidate);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Cherche l'exécutable d'un programme : chemin explicite, PATH courant, PATH du
/// shell de connexion (comme pour OpenCode), puis dossiers connus.
pub fn resolve_program(program: &str, kind: &AgentKind) -> Option<PathBuf> {
    let names = accepted_names(kind, program);
    if program.contains('/') {
        let path = PathBuf::from(&program);
        return is_executable_file(&path).then_some(path);
    }

    // Tous les candidats exécutables, dans l'ordre de recherche : PATH courant,
    // PATH du shell de connexion (les lanceurs graphiques n'en héritent pas),
    // puis dossiers connus (installations utilisateur, paquets d'application).
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push = |candidate: PathBuf| {
        if is_executable_file(&candidate) && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };
    for name in &names {
        if let Some(found) = find_in_path(name) {
            push(found);
        }
    }
    if let Some(login) = login_shell_path() {
        for dir in std::env::split_paths(&login) {
            for name in &names {
                push(dir.join(name));
            }
        }
    }
    let dirs = extra_search_dirs();
    for dir in &dirs {
        for name in &names {
            push(dir.join(name));
        }
    }

    // Plusieurs binaires du même nom peuvent coexister (ex. `dsh` de npm via
    // nvm et le paquet d'application `/opt/apps/...`) : seul celui qui répond
    // correctement à `--version` est retenu. Si aucun ne répond, le premier
    // exécutable est renvoyé pour que l'interface affiche un diagnostic clair
    // (jamais de lancement silencieux d'un faux positif).
    let mut first_existing: Option<PathBuf> = None;
    for candidate in candidates {
        if first_existing.is_none() {
            first_existing = Some(candidate.clone());
        }
        if kind == &AgentKind::Custom || probe_version(&candidate).is_some() {
            return Some(candidate);
        }
    }
    first_existing
}

/// Extrait une version `X.Y[.Z…]` d'une sortie de `--version`.
fn parse_version(text: &str) -> Option<String> {
    let mut digits = String::new();
    let mut started = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            started = true;
        } else if started && (ch == '.' || ch == '-' || ch.is_ascii_alphanumeric()) {
            digits.push(ch);
        } else if started {
            break;
        }
    }
    (digits.split('.').count() >= 2 && digits.chars().any(|c| c.is_ascii_digit()))
        .then_some(digits.trim_end_matches('.').to_string())
}

/// Lit la version d'un exécutable par `--version`, borné en temps.
///
/// Une sortie qui ne contient pas de version valide (script quelconque portant
/// le même nom) est un faux positif : la fonction renvoie `None`.
///
/// Les résultats sont mémorisés par (chemin, date de modification) : la liste
/// des agents interroge chaque adaptateur à chaque affichage, or certains CLI
/// (Node) mettent plus d'une seconde à répondre.
pub fn probe_version(path: &Path) -> Option<String> {
    use std::sync::{Mutex, OnceLock};

    type VersionCache =
        Mutex<std::collections::HashMap<(PathBuf, Option<std::time::SystemTime>), Option<String>>>;
    static CACHE: OnceLock<VersionCache> = OnceLock::new();
    let modified = std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok());
    let key = (path.to_path_buf(), modified);
    if let Some(cached) = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return cached;
    }
    let result = probe_version_uncached(path);
    if let Ok(mut cache) = CACHE
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
    {
        // Une sonde ratée n'est pas mémorisée : un exécutable en cours
        // d'installation doit pouvoir réussir plus tard.
        if result.is_some() {
            cache.insert(key, result.clone());
        }
    }
    result
}

fn probe_version_uncached(path: &Path) -> Option<String> {
    let neutral = std::env::temp_dir();
    let mut child = Command::new(path)
        .arg("--version")
        .current_dir(&neutral)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
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
    parse_version(&text)
}

/// Registre des adaptateurs configurés.
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    adapters: Vec<AgentAdapter>,
}

impl AgentRegistry {
    pub fn from_configs(configs: &[AgentConfig]) -> Self {
        Self {
            adapters: configs.iter().map(AgentAdapter::from_config).collect(),
        }
    }

    pub fn list(&self) -> Vec<AgentInfo> {
        self.adapters.iter().map(|a| a.info()).collect()
    }

    pub fn get(&self, id: &str) -> Option<&AgentAdapter> {
        self.adapters.iter().find(|a| a.id == id)
    }

    pub fn enabled(&self) -> Vec<&AgentAdapter> {
        self.adapters.iter().filter(|a| a.enabled).collect()
    }

    pub fn detected(&self) -> Vec<&AgentAdapter> {
        self.enabled()
            .into_iter()
            .filter(|a| a.detection().is_some())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(kind: AgentKind, args: Vec<&str>) -> AgentAdapter {
        AgentAdapter {
            id: "test".into(),
            kind,
            label: "Test".into(),
            program: "sh".into(),
            args: args.into_iter().map(String::from).collect(),
            enabled: true,
            timeout_secs: 30,
            env: Default::default(),
        }
    }

    #[test]
    fn claude_utilise_le_mode_impression() {
        let (program, args) =
            adapter(AgentKind::ClaudeCode, vec![]).build_command("corrige le bug");
        assert_eq!(program, "sh");
        assert_eq!(args, vec!["-p", "corrige le bug"]);
    }

    #[test]
    fn codex_utilise_exec_une_seule_fois() {
        let (_, args) = adapter(AgentKind::Codex, vec![]).build_command("x");
        assert_eq!(args, vec!["exec", "x"]);
        let (_, args) = adapter(AgentKind::Codex, vec!["exec"]).build_command("x");
        assert_eq!(args, vec!["exec", "x"]);
    }

    #[test]
    fn gemini_et_opencode() {
        let (_, args) = adapter(AgentKind::GeminiCli, vec![]).build_command("p");
        assert_eq!(args, vec!["-p", "p"]);
        let (_, args) = adapter(AgentKind::Opencode, vec![]).build_command("p");
        assert_eq!(
            args,
            vec!["run", "--format", "json", "--archived", "--standalone", "p"],
            "OpenCode reçoit son format JSON et reste hors des sessions utilisateur"
        );
        // pi : non interactif, sans session persistée.
        let (program, args) = adapter(AgentKind::Pi, vec![]).build_command("p");
        assert_eq!(program, "sh", "le helper garde son programme de test");
        assert_eq!(args, vec!["--print", "--no-session", "p"]);
        let (_, args) = adapter(AgentKind::Pi, vec!["--print"]).build_command("p");
        assert_eq!(args, vec!["--print", "--no-session", "p"], "pas de doublon");

        // Les arguments déjà fournis ne sont pas dupliqués.
        let (_, args) =
            adapter(AgentKind::Opencode, vec!["run", "--format", "json"]).build_command("p");
        assert_eq!(
            args,
            vec!["run", "--format", "json", "--archived", "--standalone", "p"]
        );
    }

    #[test]
    fn placeholder_prompt_respecte() {
        let (_, args) = adapter(AgentKind::Custom, vec!["--task", "{prompt}", "--yes"])
            .build_command("fais ceci");
        assert_eq!(args, vec!["--task", "fais ceci", "--yes"]);
    }

    #[test]
    fn detection_trouve_sh_mais_pas_un_binaire_absent() {
        let mut a = adapter(AgentKind::Custom, vec![]);
        assert!(a.detection().is_some());
        a.program = "binaire-inexistant-ih-xyz".into();
        assert!(a.detection().is_none());
        assert!(!a.info().detected);
    }

    #[test]
    fn kilo_dsh_goose_construisent_leur_commande_reelle() {
        // Kilo : NDJSON, jamais de doublon si l'utilisateur a déjà mis `run`.
        let (_, args) = adapter(AgentKind::Kilo, vec![]).build_command("corrige");
        assert_eq!(args, vec!["run", "--format", "json", "--auto", "corrige"]);
        let (_, args) = adapter(AgentKind::Kilo, vec!["run", "--format", "json", "--auto"])
            .build_command("corrige");
        assert_eq!(args, vec!["run", "--format", "json", "--auto", "corrige"]);

        // DSH : profil headless, tâche en dernier argument.
        let (_, args) = adapter(AgentKind::Dsh, vec![]).build_command("analyse");
        assert_eq!(args, vec!["--profile", "headless", "analyse"]);

        // Goose : NDJSON, sans bannière ni session persistée.
        let (_, args) = adapter(AgentKind::Goose, vec![]).build_command("explique");
        assert_eq!(
            args,
            vec![
                "run",
                "--output-format",
                "stream-json",
                "--no-session",
                "--quiet",
                "-t",
                "explique"
            ]
        );
        // Les options déjà fournies ne sont pas dupliquées.
        let (_, args) = adapter(
            AgentKind::Goose,
            vec![
                "run",
                "--output-format",
                "stream-json",
                "--no-session",
                "--quiet",
            ],
        )
        .build_command("explique");
        assert_eq!(
            args,
            vec![
                "run",
                "--output-format",
                "stream-json",
                "--no-session",
                "--quiet",
                "-t",
                "explique"
            ]
        );
    }

    #[test]
    fn les_drapeaux_d_autonomie_sont_detectes() {
        // Kilo : le drapeau est ajouté par IH (indispensable en non interactif).
        assert_eq!(
            adapter(AgentKind::Kilo, vec![]).autonomy_flags(),
            vec!["--auto".to_string()]
        );
        // Les autres familles n'élargissent rien.
        for kind in [
            AgentKind::Opencode,
            AgentKind::Pi,
            AgentKind::ClaudeCode,
            AgentKind::Codex,
            AgentKind::GeminiCli,
            AgentKind::Dsh,
            AgentKind::Goose,
        ] {
            assert!(adapter(kind, vec![]).autonomy_flags().is_empty());
        }
        // Un agent personnalisé qui reçoit un drapeau connu est signalé.
        assert_eq!(
            adapter(AgentKind::Custom, vec!["--yolo"]).autonomy_flags(),
            vec!["--yolo".to_string()]
        );
    }

    #[test]
    fn parse_version_extrait_les_formats_observes() {
        // Sorties réelles : `goose --version` (espace initial), `kilo --version`,
        // `dsh --version`.
        assert_eq!(parse_version(" 1.61.0\n").as_deref(), Some("1.61.0"));
        assert_eq!(parse_version("7.8.32").as_deref(), Some("7.8.32"));
        assert_eq!(parse_version("0.1.0-rc.7").as_deref(), Some("0.1.0-rc.7"));
        assert_eq!(parse_version("pas une version"), None);
    }

    #[test]
    fn resolution_par_alias_et_dossier_hors_path() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ih-agents-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dossier");
        // Alias réel : `kilocode` est un lien vers `kilo` sur les installations
        // npm de @kilocode/cli. Ici deux scripts qui répondent à --version.
        let alias = dir.join("kilo");
        let mut file = std::fs::File::create(&alias).expect("script");
        writeln!(file, "#!/bin/sh").unwrap();
        writeln!(
            file,
            "if [ \"$1\" = \"--version\" ]; then echo 7.8.32; exit 0; fi"
        )
        .unwrap();
        writeln!(file, "echo ok").unwrap();
        drop(file);
        std::fs::set_permissions(&alias, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let alias2 = dir.join("kilocode");
        std::fs::copy(&alias, &alias2).expect("alias");
        std::fs::set_permissions(&alias2, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let mut adapter = adapter(AgentKind::Kilo, vec![]);
        adapter.program = "kilo".into();
        // Le dossier est ajouté à PATH le temps du test : c'est le scénario
        // « dossier absent du PATH graphique mais trouvé par IH ».
        let previous = std::env::var_os("PATH");
        let mut value = std::ffi::OsString::from(&dir);
        if let Some(existing) = &previous {
            value.push(":");
            value.push(existing);
        }
        std::env::set_var("PATH", &value);
        let resolved = adapter.detection().expect("binaire résolu");
        assert_eq!(resolved.parent().unwrap(), dir.as_path());
        assert_eq!(probe_version(&resolved).as_deref(), Some("7.8.32"));
        match previous {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn programme_absent_signale_avec_un_diagnostic() {
        // Un agent personnalisé n'a pas d'alias : un nom absent reste absent.
        let mut adapter = adapter(AgentKind::Custom, vec![]);
        adapter.program = "binaire-inexistant-ih-xyz".into();
        let info = adapter.info();
        assert!(!info.detected);
        assert!(info.error.is_some());
        assert!(info.version.is_none());
    }

    #[test]
    fn alias_de_famille_resolu_meme_si_le_nom_configuré_diffère() {
        // Si la configuration nomme un binaire absent mais que la famille est
        // connue, IH retrouve le binaire réel (alias documentés par famille).
        let mut adapter = adapter(AgentKind::Goose, vec![]);
        adapter.program = "goose-cli-nom-local-absent".into();
        if resolve_program("goose", &AgentKind::Goose).is_some() {
            assert!(adapter.detection().is_some(), "alias goose retrouvé");
        }
    }

    #[test]
    fn registre_filtre_les_actifs() {
        let configs = vec![
            AgentConfig {
                id: "a".into(),
                kind: AgentKind::ClaudeCode,
                label: "A".into(),
                program: "sh".into(),
                enabled: true,
                ..Default::default()
            },
            AgentConfig {
                id: "b".into(),
                kind: AgentKind::Codex,
                label: "B".into(),
                program: "sh".into(),
                enabled: false,
                ..Default::default()
            },
        ];
        let registry = AgentRegistry::from_configs(&configs);
        assert_eq!(registry.list().len(), 2);
        assert_eq!(registry.enabled().len(), 1);
        assert_eq!(registry.detected().len(), 1);
    }
}
