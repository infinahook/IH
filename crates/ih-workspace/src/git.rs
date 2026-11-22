//! Opérations Git réelles, exécutées via le binaire `git`.
//!
//! Aucune commande ne passe par un shell : les arguments sont toujours fournis
//! séparément à [`std::process::Command`]. Les chemins reçus de l'interface
//! sont validés avant d'être transmis (refus de `..`, des chemins absolus et
//! des préfixes `-` qui pourraient être interprétés comme des options).
//!
//! Limites assumées : pas de merge, rebase, cherry-pick ni gestion de conflits ;
//! [`GitRepo::diff`] ne renvoie que du texte (diff binaire non décodé).

use ih_core::{IhError, IhResult};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use crate::MAX_DIFF_BYTES;

/// Dépôt Git découvert depuis un répertoire de projet.
#[derive(Debug, Clone)]
pub struct GitRepo {
    root: PathBuf,
}

/// État complet d'un dépôt, prêt pour l'affichage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub entries: Vec<StatusEntry>,
    pub ahead: u32,
    pub behind: u32,
    pub clean: bool,
}

/// Une entrée de `git status --porcelain=v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry {
    /// Chemin relatif à la racine du dépôt (renommages : nouveau chemin).
    pub path: String,
    /// Code de la colonne index (`A`, `M`, `D`, `R`, `?`, …).
    pub index: char,
    /// Code de la colonne arbre de travail.
    pub worktree: char,
    /// Vrai si la modification est dans l'index.
    pub staged: bool,
}

/// Un commit tel que présenté dans l'historique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// Un arbre de travail Git (`git worktree list --porcelain`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    pub bare: bool,
}

impl GitRepo {
    /// Découvre le dépôt contenant `path`.
    pub fn discover(path: &Path) -> IhResult<Self> {
        if !path.exists() {
            return Err(IhError::NotFound(format!(
                "chemin inexistant : {}",
                path.display()
            )));
        }
        let start = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."))
        };

        let mut command = base_command();
        command
            .arg("-C")
            .arg(&start)
            .args(["rev-parse", "--show-toplevel"]);
        let output = command.output().map_err(spawn_error)?;

        if !output.status.success() {
            return Err(IhError::Unsupported(format!(
                "pas un dépôt Git : {}",
                start.display()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let top = stdout.trim();
        if top.is_empty() {
            return Err(IhError::Unsupported(format!(
                "pas un dépôt Git : {}",
                start.display()
            )));
        }
        let root = std::fs::canonicalize(top)
            .map_err(|e| IhError::Internal(format!("racine Git invalide ({top}) : {e}")))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Branche courante, `HEAD` si l'état est détaché.
    pub fn current_branch(&self) -> IhResult<String> {
        if let Ok(output) = self.output(&["rev-parse", "--abbrev-ref", "HEAD"]) {
            if output.status.success() {
                let branch = text(&output.stdout);
                if !branch.is_empty() {
                    return Ok(branch);
                }
            }
        }
        // Dépôt sans commit : rev-parse échoue mais symbolic-ref connaît la
        // branche encore vide.
        if let Ok(output) = self.output(&["symbolic-ref", "--short", "HEAD"]) {
            if output.status.success() {
                let branch = text(&output.stdout);
                if !branch.is_empty() {
                    return Ok(branch);
                }
            }
        }
        Ok("HEAD".to_string())
    }

    /// État du dépôt, entrées structurées comprises.
    pub fn status(&self) -> IhResult<GitStatus> {
        let output = self.checked(&["status", "--porcelain=v1", "-z", "--branch"])?;
        let tokens: Vec<&[u8]> = output.stdout.split(|byte| *byte == 0).collect();

        let mut branch = String::new();
        let mut ahead = 0;
        let mut behind = 0;
        let mut entries = Vec::new();
        let mut index = 0;

        while index < tokens.len() {
            let token = tokens[index];
            if token.is_empty() {
                index += 1;
                continue;
            }
            if let Some(header) = token.strip_prefix(b"## ") {
                let (parsed_branch, parsed_ahead, parsed_behind) = parse_status_header(header);
                branch = parsed_branch;
                ahead = parsed_ahead;
                behind = parsed_behind;
                index += 1;
                continue;
            }
            if token.len() < 3 {
                index += 1;
                continue;
            }
            let index_code = token[0] as char;
            let worktree_code = token[1] as char;
            let path = if token[2] == b' ' {
                &token[3..]
            } else {
                &token[2..]
            };
            entries.push(StatusEntry {
                path: String::from_utf8_lossy(path).into_owned(),
                index: index_code,
                worktree: worktree_code,
                staged: !matches!(index_code, ' ' | '?' | '!'),
            });
            // `-z` ajoute le chemin d'origine après le nouveau pour R/C.
            index += if matches!(index_code, 'R' | 'C') {
                2
            } else {
                1
            };
        }

        if branch.is_empty() {
            branch = self.current_branch().unwrap_or_else(|_| "HEAD".to_string());
        }

        let clean = entries.is_empty();
        Ok(GitStatus {
            branch,
            entries,
            ahead,
            behind,
            clean,
        })
    }

    /// Diff borné (2 Mio) entre l'index et l'arbre de travail, ou entre
    /// `HEAD` et l'index si `staged`.
    pub fn diff(&self, staged: bool, path: Option<&str>) -> IhResult<String> {
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "--no-textconv"];
        if staged {
            args.push("--cached");
        }
        if let Some(path) = path {
            validate_repo_path(path)?;
            args.push("--");
            args.push(path);
        }
        let output = self.checked(&args)?;
        Ok(truncate_utf8(&output.stdout, MAX_DIFF_BYTES))
    }

    /// Ajoute les chemins donnés à l'index.
    pub fn stage(&self, paths: &[String]) -> IhResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        for path in paths {
            validate_repo_path(path)?;
        }
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.checked(&args)?;
        Ok(())
    }

    /// Retire les chemins donnés de l'index.
    pub fn unstage(&self, paths: &[String]) -> IhResult<()> {
        if paths.is_empty() {
            return Ok(());
        }
        for path in paths {
            validate_repo_path(path)?;
        }
        let mut args = vec!["reset", "HEAD", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.checked(&args)?;
        Ok(())
    }

    /// Crée un commit et renvoie son hash court.
    pub fn commit(&self, message: &str) -> IhResult<String> {
        if message.trim().is_empty() {
            return Err(IhError::Invalid("message de commit vide".to_string()));
        }
        self.checked(&["commit", "-m", message])?;
        let output = self.checked(&["rev-parse", "--short", "HEAD"])?;
        let hash = text(&output.stdout);
        if hash.is_empty() {
            return Err(IhError::Internal(
                "git n'a pas renvoyé de hash après le commit".to_string(),
            ));
        }
        Ok(hash)
    }

    /// Branches locales.
    pub fn branches(&self) -> IhResult<Vec<String>> {
        let output = self.checked(&["for-each-ref", "--format=%(refname:short)", "refs/heads"])?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Derniers commits, du plus récent au plus ancien.
    pub fn log(&self, limit: u32) -> IhResult<Vec<CommitInfo>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        // Un dépôt fraîchement initialisé n'a pas de HEAD : historique vide.
        let head = self.output(&["rev-parse", "--verify", "--quiet", "HEAD"])?;
        if !head.status.success() {
            return Ok(Vec::new());
        }

        // Garde-fou mémoire côté IH ; 1000 commits suffisent à l'interface.
        let limit = limit.min(1_000);
        let count = format!("-n{limit}");
        let format = "--format=%h%x1f%an%x1f%aI%x1f%s%x1e";
        let output = self.checked(&["log", &count, format])?;
        let raw = String::from_utf8_lossy(&output.stdout);
        let mut commits = Vec::new();
        for record in raw.split('\u{1e}') {
            let record = record.trim_start_matches(['\n', '\r']);
            if record.is_empty() {
                continue;
            }
            let mut fields = record.splitn(4, '\u{1f}');
            let (Some(hash), Some(author), Some(date), Some(subject)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            // `%aI` est déjà ISO 8601 ; chrono normalise l'offset pour que
            // l'interface n'ait jamais à interpréter deux formats.
            let date = chrono::DateTime::parse_from_rfc3339(date)
                .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_else(|_| date.to_string());
            commits.push(CommitInfo {
                hash: hash.to_string(),
                author: author.to_string(),
                date,
                subject: subject.to_string(),
            });
        }
        Ok(commits)
    }

    /// Arbres de travail déclarés, y compris le principal.
    pub fn worktrees(&self) -> IhResult<Vec<Worktree>> {
        let output = self.checked(&["worktree", "list", "--porcelain"])?;
        let raw = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();

        for block in raw.split("\n\n") {
            let mut path = None;
            let mut head = String::new();
            let mut branch = None;
            let mut bare = false;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(value.to_string());
                } else if let Some(value) = line.strip_prefix("HEAD ") {
                    head = value.to_string();
                } else if let Some(value) = line.strip_prefix("branch ") {
                    branch = Some(
                        value
                            .strip_prefix("refs/heads/")
                            .unwrap_or(value)
                            .to_string(),
                    );
                } else if line == "bare" {
                    bare = true;
                }
            }
            if let Some(path) = path {
                worktrees.push(Worktree {
                    path,
                    head,
                    branch,
                    bare,
                });
            }
        }
        Ok(worktrees)
    }

    /// Ajoute un arbre de travail. Si `create`, la branche est créée depuis
    /// `HEAD` ; sinon elle doit déjà exister.
    pub fn add_worktree(&self, target: &Path, branch: &str, create: bool) -> IhResult<()> {
        let target = target
            .to_str()
            .ok_or_else(|| IhError::Invalid("chemin d'arbre de travail non UTF-8".to_string()))?;
        if target.trim().is_empty() {
            return Err(IhError::Invalid(
                "chemin d'arbre de travail vide".to_string(),
            ));
        }
        if target.starts_with('-') {
            return Err(IhError::Invalid(format!(
                "chemin d'arbre de travail refusé (préfixe `-`) : {target}"
            )));
        }
        validate_branch(branch)?;

        if create {
            self.checked(&["worktree", "add", "-b", branch, "--", target])?;
        } else {
            self.checked(&["worktree", "add", "--", target, branch])?;
        }
        Ok(())
    }

    /// Retire un arbre de travail (avec `--force` si demandé).
    pub fn remove_worktree(&self, target: &Path, force: bool) -> IhResult<()> {
        let target = target
            .to_str()
            .ok_or_else(|| IhError::Invalid("chemin d'arbre de travail non UTF-8".to_string()))?;
        if target.trim().is_empty() {
            return Err(IhError::Invalid(
                "chemin d'arbre de travail vide".to_string(),
            ));
        }
        if target.starts_with('-') {
            return Err(IhError::Invalid(format!(
                "chemin d'arbre de travail refusé (préfixe `-`) : {target}"
            )));
        }
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push("--");
        args.push(target);
        self.checked(&args)?;
        Ok(())
    }

    /// Exécute `git` sans vérifier le code de sortie.
    fn output(&self, args: &[&str]) -> IhResult<Output> {
        base_command()
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .map_err(spawn_error)
    }

    /// Exécute `git` et transforme un échec en erreur IH explicite.
    fn checked(&self, args: &[&str]) -> IhResult<Output> {
        let output = self.output(args)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(command_error(args, &output))
        }
    }
}

/// Commande `git` configurée pour un comportement stable et non interactif.
fn base_command() -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        // Messages stables (pas de localisation) pour les erreurs et le statut.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn spawn_error(error: std::io::Error) -> IhError {
    if error.kind() == std::io::ErrorKind::NotFound {
        IhError::Unsupported("binaire `git` introuvable dans PATH".to_string())
    } else {
        IhError::Internal(format!("exécution de git impossible : {error}"))
    }
}

fn command_error(args: &[&str], output: &Output) -> IhError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        format!("code de sortie {}", output.status)
    } else {
        stderr.to_string()
    };
    IhError::Invalid(format!(
        "git {} a échoué : {detail}",
        args.first().copied().unwrap_or("?")
    ))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Refuse tout chemin qui pourrait être interprété comme une option ou sortir
/// du dépôt. Les arguments ne passent jamais par un shell, mais `--` et les
/// options Git restent une surface d'injection.
fn validate_repo_path(path: &str) -> IhResult<()> {
    if path.is_empty() {
        return Err(IhError::Invalid("chemin Git vide".to_string()));
    }
    if path.starts_with('-') {
        return Err(IhError::Invalid(format!(
            "chemin Git refusé (préfixe `-`) : {path}"
        )));
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(IhError::Invalid(format!(
            "chemin Git absolu refusé : {path}"
        )));
    }
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(IhError::Invalid(format!(
            "chemin Git contenant `..` refusé : {path}"
        )));
    }
    Ok(())
}

/// Validation conservatrice d'un nom de branche (en plus de `git` lui-même).
fn validate_branch(branch: &str) -> IhResult<()> {
    let trimmed = branch.trim();
    let invalid = trimmed.is_empty()
        || trimmed.starts_with('-')
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.ends_with('.')
        || trimmed.ends_with(".lock")
        || trimmed.contains("..")
        || trimmed.contains("@{")
        || trimmed
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c));
    if invalid {
        return Err(IhError::Invalid(format!(
            "nom de branche invalide : {branch}"
        )));
    }
    Ok(())
}

fn parse_status_header(header: &[u8]) -> (String, u32, u32) {
    let raw = String::from_utf8_lossy(header);
    let raw = raw.trim().trim_end_matches('\u{0}');
    let (ahead, behind) = parse_ahead_behind(raw);

    let name = if let Some(rest) = raw.strip_prefix("No commits yet on ") {
        rest
    } else if raw.starts_with("HEAD (no branch)") {
        "HEAD"
    } else {
        raw.split("...").next().unwrap_or(raw)
    };
    (name.trim().to_string(), ahead, behind)
}

fn parse_ahead_behind(header: &str) -> (u32, u32) {
    let Some(start) = header.find('[') else {
        return (0, 0);
    };
    let Some(relative_end) = header[start..].find(']') else {
        return (0, 0);
    };
    let inner = &header[start + 1..start + relative_end];

    let mut ahead = 0;
    let mut behind = 0;
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("ahead ") {
            ahead = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = part.strip_prefix("behind ") {
            behind = value.trim().parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

/// Tronque sur une frontière UTF-8 valide et signale la coupure.
fn truncate_utf8(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut cut = limit;
    while cut > 0 && (bytes[cut] & 0xC0) == 0x80 {
        cut -= 1;
    }
    let head = String::from_utf8_lossy(&bytes[..cut]);
    format!(
        "{head}\n… [diff tronqué : {limit} octets affichés sur {}] …\n",
        bytes.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git doit être installé pour les tests");
        assert!(
            output.status.success(),
            "git {args:?} a échoué : {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo() -> (TempDir, GitRepo) {
        let dir = TempDir::new("git");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "IH Test"]);
        git(dir.path(), &["config", "user.email", "test@ih.local"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        let repo = GitRepo::discover(dir.path()).expect("dépôt valide");
        (dir, repo)
    }

    fn commit_fichier(repo: &GitRepo, name: &str, body: &str) -> String {
        std::fs::write(repo.root().join(name), body).expect("écriture du fichier");
        repo.stage(&[name.to_string()]).expect("indexation");
        repo.commit(&format!("ajout {name}"))
            .expect("création du commit")
    }

    #[test]
    fn decouverte_et_refus_d_un_dossier_non_git() {
        let dir = TempDir::new("nongit");
        let error = GitRepo::discover(dir.path()).expect_err("pas un dépôt");
        assert!(matches!(error, IhError::Unsupported(_)));
        assert!(error.to_string().contains("pas un dépôt Git"));

        let (_tmp, repo) = init_repo();
        assert_eq!(repo.root(), repo.root().canonicalize().unwrap().as_path());

        let file = repo.root().join("depuis-fichier.txt");
        std::fs::write(&file, "x").unwrap();
        let from_file = GitRepo::discover(&file).expect("découverte depuis un fichier");
        assert_eq!(from_file.root(), repo.root());
    }

    #[test]
    fn statut_propre_puis_entree_non_suivie() {
        let (_tmp, repo) = init_repo();
        assert_eq!(repo.current_branch().unwrap(), "main");

        let status = repo.status().unwrap();
        assert!(status.clean);
        assert_eq!(status.branch, "main");
        assert!(status.entries.is_empty());

        std::fs::write(repo.root().join("a.txt"), "un\n").unwrap();
        let status = repo.status().unwrap();
        assert!(!status.clean);
        assert_eq!(status.entries.len(), 1);
        assert_eq!(status.entries[0].path, "a.txt");
        assert_eq!(status.entries[0].index, '?');
        assert!(!status.entries[0].staged);
    }

    #[test]
    fn stage_commit_et_log_retrouve_le_hash() {
        let (_tmp, repo) = init_repo();
        std::fs::write(repo.root().join("a.txt"), "bonjour\n").unwrap();
        repo.stage(&["a.txt".to_string()]).unwrap();

        let staged = repo.status().unwrap();
        assert_eq!(staged.entries[0].index, 'A');
        assert!(staged.entries[0].staged);

        let hash = repo.commit("premier commit").unwrap();
        assert!(!hash.is_empty());
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(repo.status().unwrap().clean);

        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].hash, hash);
        assert_eq!(log[0].author, "IH Test");
        assert_eq!(log[0].subject, "premier commit");
        assert!(log[0].date.contains('T'));
        assert!(repo.branches().unwrap().contains(&"main".to_string()));

        assert!(matches!(repo.commit("   "), Err(IhError::Invalid(_))));
    }

    #[test]
    fn diff_staged_et_non_staged() {
        let (_tmp, repo) = init_repo();
        let hash = commit_fichier(&repo, "a.txt", "ligne un\n");
        assert!(!hash.is_empty());

        std::fs::write(repo.root().join("a.txt"), "ligne un\nligne deux\n").unwrap();
        let unstaged = repo.diff(false, None).unwrap();
        assert!(unstaged.contains("+ligne deux"));
        assert!(repo.diff(true, None).unwrap().is_empty());

        repo.stage(&["a.txt".to_string()]).unwrap();
        let staged = repo.diff(true, Some("a.txt")).unwrap();
        assert!(staged.contains("+ligne deux"));
        assert!(repo.diff(false, None).unwrap().is_empty());
        assert!(repo.diff(false, Some("absent.txt")).unwrap().is_empty());

        repo.unstage(&["a.txt".to_string()]).unwrap();
        assert!(!repo.status().unwrap().entries[0].staged);
        assert!(repo.diff(false, None).unwrap().contains("+ligne deux"));
        assert!(repo.diff(true, None).unwrap().is_empty());
    }

    #[test]
    fn chemins_d_injection_refuses() {
        let (_tmp, repo) = init_repo();
        assert!(repo.stage(&["../evil".to_string()]).is_err());
        assert!(repo.stage(&["-n".to_string()]).is_err());
        assert!(repo.stage(&["/etc/passwd".to_string()]).is_err());
        assert!(repo.unstage(&["--hard".to_string()]).is_err());
        assert!(repo.diff(false, Some("../x")).is_err());
        assert!(repo.diff(true, Some("-p")).is_err());
        assert!(repo.add_worktree(Path::new("-x"), "main", false).is_err());
        assert!(repo
            .add_worktree(Path::new("/tmp/x-wt"), "bad branch", true)
            .is_err());
    }

    #[test]
    fn worktrees_ajout_puis_retrait() {
        let (_tmp, repo) = init_repo();
        commit_fichier(&repo, "a.txt", "contenu\n");

        let parent = TempDir::new("worktree");
        let target = parent.path().join("feature-wt");
        repo.add_worktree(&target, "feature", true).unwrap();
        assert!(target.join("a.txt").exists());

        let worktrees = repo.worktrees().unwrap();
        assert_eq!(worktrees.len(), 2);
        assert!(worktrees
            .iter()
            .any(|tree| tree.branch.as_deref() == Some("feature")));
        assert!(repo.branches().unwrap().contains(&"feature".to_string()));

        repo.remove_worktree(&target, true).unwrap();
        assert!(!target.exists());
        assert_eq!(repo.worktrees().unwrap().len(), 1);

        // Réutilise la branche existante sans `create`, puis retire sans force.
        let existing = parent.path().join("feature-existing");
        repo.add_worktree(&existing, "feature", false).unwrap();
        assert!(existing.join("a.txt").exists());
        repo.remove_worktree(&existing, false).unwrap();
        assert!(!existing.exists());
    }

    #[test]
    fn entetes_de_statut_et_troncature() {
        assert_eq!(
            parse_status_header(b"main...origin/main [ahead 2, behind 1]"),
            ("main".to_string(), 2, 1)
        );
        assert_eq!(
            parse_status_header(b"No commits yet on main"),
            ("main".to_string(), 0, 0)
        );
        assert_eq!(
            parse_status_header(b"HEAD (no branch)"),
            ("HEAD".to_string(), 0, 0)
        );
        assert_eq!(
            parse_status_header(b"dev...origin/dev [behind 3]"),
            ("dev".to_string(), 0, 3)
        );

        let long = "a".repeat(32);
        let truncated = truncate_utf8(long.as_bytes(), 8);
        assert!(truncated.starts_with("aaaaaaaa"));
        assert!(truncated.contains("tronqué"));
        assert!(truncated.contains("32"));

        // La coupe ne doit pas casser un caractère multioctet.
        let accents = "ééé";
        let truncated = truncate_utf8(accents.as_bytes(), 3);
        assert!(truncated.starts_with("é"));
        assert!(truncated.contains("tronqué"));
        assert!(!truncated.contains('\u{FFFD}'));
    }

    #[test]
    fn depot_vide_et_head_detache() {
        let (dir, repo) = init_repo();
        let status = repo.status().unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(repo.log(5).unwrap().len(), 0);

        commit_fichier(&repo, "a.txt", "un\n");
        git(dir.path(), &["checkout", "-q", "--detach", "HEAD"]);
        assert_eq!(repo.current_branch().unwrap(), "HEAD");
        assert_eq!(repo.status().unwrap().branch, "HEAD");
    }
}
