//! Test E — NV4 : les inspections Git côté hôte sur un worktree contrôlé par
//! l'agent ne doivent jamais exécuter un programme choisi par le dépôt.
//!
//! Le dépôt jette un `core.fsmonitor` et un pilote de conversion `textconv`
//! malveillants ; `changed_files` / `collect_changes` doivent les neutraliser.
//! Test non ignoré : aucune sandbox n'est nécessaire (inspection hôte seule).

use ih_agent_mode::{changed_files, collect_changes, create_sandbox, remove_sandbox};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn base(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ih-hostgit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("GIT_AUTHOR_NAME", "IH")
        .env("GIT_AUTHOR_EMAIL", "ih@test")
        .env("GIT_COMMITTER_NAME", "IH")
        .env("GIT_COMMITTER_EMAIL", "ih@test")
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?} : {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn marqueur(path: &Path, name: &str) -> String {
    format!(
        "#!/bin/sh\ntouch '{}'\nprintf '1: README.md\\0'\n",
        path.join(name).display()
    )
}

fn ecrire_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn aucune_execution_controlee_par_le_depot_lors_de_l_inspection() {
    let base = base("fsmonitor");
    let repo = base.join("depot");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# projet\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    let sandbox = create_sandbox(&repo, &base.join("wt-base"), "host-1").expect("worktree");
    let workdir = sandbox.workdir().to_path_buf();

    // Espions : fsmonitor et textconv tentent de créer des canaris.
    let espion_fsmonitor = base.join("fsmonitor.sh");
    ecrire_executable(&espion_fsmonitor, &marqueur(&base, "CANARI-FSMONITOR"));
    let espion_textconv = base.join("textconv.sh");
    ecrire_executable(&espion_textconv, &marqueur(&base, "CANARI_TEXTCONV"));

    // Configuration malveillante du dépôt (ce que ferait un agent avant la
    // correction, ou un dépôt piégé fourni avec un projet).
    git(
        &repo,
        &[
            "config",
            "core.fsmonitor",
            espion_fsmonitor.to_str().unwrap(),
        ],
    );
    git(
        &repo,
        &[
            "config",
            "diff.evil.textconv",
            espion_textconv.to_str().unwrap(),
        ],
    );
    git(&repo, &["config", "diff.evil.binary", "true"]);
    std::fs::write(workdir.join(".gitattributes"), "*.md diff=evil\n").unwrap();
    std::fs::write(workdir.join("README.md"), "# projet modifié\n").unwrap();

    // Preuve du mécanisme : un `git status` non durci exécute bien l'espion.
    let brut = Command::new("git")
        .args(["-C", workdir.to_str().unwrap(), "status", "--porcelain"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git");
    assert!(brut.success());
    assert!(
        base.join("CANARI-FSMONITOR").exists(),
        "le scénario de test doit prouver l'exécution non durcie"
    );
    let _ = std::fs::remove_file(base.join("CANARI-FSMONITOR"));

    // Inspection durcie d'IH : aucun espion ne doit être lancé.
    let files = changed_files(&sandbox).expect("changed_files");
    assert!(files.iter().any(|path| path == "README.md"), "{files:?}");
    let changes = collect_changes(&sandbox, 64 * 1024).expect("collect_changes");
    assert!(!changes.is_empty());
    assert!(
        !base.join("CANARI-FSMONITOR").exists(),
        "core.fsmonitor a été exécuté par l'inspection d'IH"
    );
    assert!(
        !base.join("CANARI_TEXTCONV").exists(),
        "textconv a été exécuté par l'inspection d'IH"
    );

    // Nettoyage : la configuration piégée appartient au dépôt jetable.
    remove_sandbox(&sandbox).ok();
    let _ = std::fs::remove_dir_all(&base);
}
