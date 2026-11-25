//! Git utilisable dans un worktree sandboxé, sans écriture du dépôt principal.
//!
//! Le worktree est créé par IH (`git worktree add --detach`) ; son fichier
//! `.git` pointe vers `<dépôt>/.git/worktrees/<nom>`. La sandbox monte ce
//! dossier de métadonnées en écriture et le répertoire Git commun en lecture
//! seule : `git status`/`git diff` fonctionnent, mais le dépôt reste protégé.
//!
//! Test marqué `#[ignore]` (bubblewrap requis) : exécution explicite avec
//! `cargo test -p ih-agents --test sandbox_git -- --ignored`.

use ih_agents::sandbox::{self, SecurityLevel};
use ih_core::config::AgentKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn ready() -> bool {
    sandbox::availability().functional
}

fn require_sandbox() {
    if std::env::var("IH_REQUIRE_SANDBOX").ok().as_deref() == Some("1") && !ready() {
        panic!(
            "IH_REQUIRE_SANDBOX=1 mais la sandbox n'est pas fonctionnelle : {:?}",
            sandbox::availability()
        );
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

fn unique(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ih-sbxgit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn run_sandbox(workdir: &Path, git: &sandbox::GitMounts, script: &str) -> (bool, String, String) {
    let plan = sandbox::sandboxed_command(
        SecurityLevel::Isolated,
        &AgentKind::Custom,
        Path::new("/bin/sh"),
        &["-c".to_string(), script.to_string()],
        workdir,
        &[],
        "git-test",
        Some(git),
    )
    .expect("plan shell");
    let output = Command::new(&plan.program)
        .args(&plan.args)
        .env_clear()
        .envs(plan.env.iter().cloned())
        .stdin(Stdio::null())
        .output()
        .expect("exécution sandbox");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn git_status_et_diff_fonctionnent_dans_le_worktree_sans_toucher_le_depot() {
    require_sandbox();
    if !ready() {
        eprintln!("IGNORÉ: bubblewrap non fonctionnel sur cette machine");
        return;
    }

    let root = unique("repo");
    let base = root.clone();
    let repo = base.join("depot");
    let worktree = base.join("wt");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# projet\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "ih@test"]);
    git(&repo, &["config", "user.name", "IH"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            worktree.to_str().unwrap(),
        ],
    );
    let common = std::fs::canonicalize(repo.join(".git")).unwrap();
    let git = sandbox::GitMounts::expected(&common, "wt", &worktree).expect("montages Git IH");
    let gitdir = git.worktree_gitdir.clone();
    // Le canari de protection : une écriture du dépôt doit échouer.
    let config_path = repo.join(".git/config");
    let before = std::fs::read_to_string(&config_path).unwrap();

    let script = format!(
        "cd {wt} && \
         echo 'modification' >> README.md && echo MODIFIED=ok; \
         echo STATUT=$(git status --porcelain | head -2); \
         echo DIFF=$(git diff --stat | tail -1); \
         echo INDEX=$(printenv GIT_INDEX_FILE); \
         echo PIRATE=1 >> {config} 2>&1 && echo DEPOT_WRITE=ok || echo DEPOT_WRITE=refused; \
         echo PIRATE=1 >> {gitdir}/HEAD 2>&1 && echo GITDIR_WRITE=ok || echo GITDIR_WRITE=refused; \
         echo PIRATE=1 >> {gitdir}/commondir 2>&1 && echo COMMONDIR_WRITE=ok || echo COMMONDIR_WRITE=refused",
        wt = worktree.display(),
        config = config_path.display(),
        gitdir = gitdir.display()
    );
    let (success, stdout, stderr) = run_sandbox(&worktree, &git, &script);
    println!("stdout={stdout}\nstderr={stderr}");
    assert!(success, "sandbox: {stderr}");
    assert!(stdout.contains("MODIFIED=ok"), "écriture dans le worktree");
    assert!(
        stdout.contains("README.md"),
        "git status doit voir la modification : {stdout}"
    );
    assert!(
        stdout.contains("DIFF="),
        "git diff doit fonctionner : {stdout}"
    );
    assert!(
        stdout.contains("INDEX=/home/agent/.git-index"),
        "l'index de session vit dans le home privé : {stdout}"
    );
    // Le dépôt et les métadonnées Git sont en lecture seule dans la sandbox.
    assert!(stdout.contains("DEPOT_WRITE=refused"), "{stdout}");
    assert!(stdout.contains("GITDIR_WRITE=refused"), "{stdout}");
    assert!(stdout.contains("COMMONDIR_WRITE=refused"), "{stdout}");
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        before,
        "le .git/config du dépôt a été modifié"
    );

    // Un `.git` empoisonné APRÈS la session ne produit aucun montage : la
    // construction suivante est refusée (NV1).
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", base.join("hors-depot").display()),
    )
    .unwrap();
    assert!(
        git.verify(&worktree).is_err(),
        "`.git` détourné doit être refusé"
    );

    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", worktree.to_str().unwrap()])
        .current_dir(&repo)
        .status();
    let _ = std::fs::remove_dir_all(&root);
}
