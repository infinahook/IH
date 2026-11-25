//! NV1/NV2 : le fichier `.git` et `commondir` d'un worktree sont inscriptibles
//! par l'agent ; ils ne doivent jamais faire autorité pour construire des
//! montages de sandbox.
//!
//! Ces tests utilisent des dépôts et des HOME factices uniquement.

use ih_agents::sandbox::{self, GitMounts, SecurityLevel};
use ih_core::config::AgentKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn base(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ih-git-audit-{tag}-{}", std::process::id()));
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

/// Dépôt vide + worktree détaché dans `base/wt`.
fn depot(base: &Path) -> (PathBuf, PathBuf) {
    let repo = base.join("depot");
    let wt = base.join("wt");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# projet\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    std::fs::create_dir_all(&wt).unwrap();
    git(
        &repo,
        &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
    );
    (repo, wt)
}

fn common_dir(repo: &Path) -> PathBuf {
    std::fs::canonicalize(repo.join(".git")).unwrap()
}

/// Test A (sans bubblewrap) : un `.git` mensonger empêche toute construction de
/// mounts, et aucun chemin de l'hôte n'apparaît dans les arguments.
#[test]
fn un_fichier_git_modifie_est_refuse_sans_montage() {
    let base = base("git");
    let (repo, wt) = depot(&base);
    let common = common_dir(&repo);
    let canary = base.join("canary-home");
    std::fs::create_dir_all(canary.join(".ssh")).unwrap();
    std::fs::write(canary.join(".ssh/id_rsa"), "CANARI-SSH\n").unwrap();

    // Référence saine : les montages attendus sont calculés par IH.
    let mounts = GitMounts::expected(&common, "wt", &wt).expect("worktree sain");
    assert_eq!(mounts.common_dir, common);
    assert_eq!(mounts.worktree_gitdir, common.join("worktrees").join("wt"));

    // Attaque 1 : `.git` pointe vers un autre répertoire de l'hôte.
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", canary.display())).unwrap();
    let error = mounts
        .verify(&wt)
        .expect_err("`.git` détourné doit être refusé");
    assert!(
        format!("{error}").contains("modifié"),
        "diagnostic : {error}"
    );
    // Aucun montage n'est produit : la construction de la sandbox refuse.
    let refused = sandbox::sandboxed_command(
        SecurityLevel::Isolated,
        &AgentKind::Custom,
        Path::new("/bin/sh"),
        &["-c".to_string(), "true".to_string()],
        &wt,
        &[],
        "git-audit",
        Some(&mounts),
    );
    assert!(
        refused.is_err(),
        "aucun plan de sandbox ne doit être produit"
    );

    // Attaque 2 : `.git` devient un lien symbolique vers un fichier quelconque.
    let elsewhere = base.join("gitdir.txt");
    std::fs::write(&elsewhere, format!("gitdir: {}\n", canary.display())).unwrap();
    let _ = std::fs::remove_file(wt.join(".git"));
    std::os::unix::fs::symlink(&elsewhere, wt.join(".git")).unwrap();
    let error = mounts
        .verify(&wt)
        .expect_err("`.git` symbolique doit être refusé");
    assert!(
        format!("{error}").contains("fichier régulier"),
        "diagnostic : {error}"
    );

    // Attaque 3 : contenu malformé / relatif / absent.
    let _ = std::fs::remove_file(wt.join(".git"));
    std::fs::write(wt.join(".git"), "pas un pointeur git\n").unwrap();
    assert!(mounts.verify(&wt).is_err(), "`.git` malformé refusé");
    std::fs::write(wt.join(".git"), "gitdir: ../ailleurs\n").unwrap();
    assert!(mounts.verify(&wt).is_err(), "`.git` relatif refusé");
    let _ = std::fs::remove_file(wt.join(".git"));
    assert!(mounts.verify(&wt).is_err(), "`.git` absent refusé");

    // Le canari de l'hôte n'a jamais été touché.
    assert!(canary.join(".ssh/id_rsa").exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// Test B (sans bubblewrap) : `commondir` mensonger est refusé, quel que soit
/// le chemin visé ou sa forme.
#[test]
fn un_commondir_detourne_est_refuse() {
    let base = base("commondir");
    let (repo, wt) = depot(&base);
    let common = common_dir(&repo);
    let mounts = GitMounts::expected(&common, "wt", &wt).expect("worktree sain");
    let gitdir = mounts.worktree_gitdir.clone();
    let home_canary = base.join("faux-home");
    std::fs::create_dir_all(home_canary.join(".ssh")).unwrap();
    let ssh_canary = home_canary.join(".ssh");

    // Cas valide : `commondir` d'origine fonctionne.
    assert!(mounts.verify(&wt).is_ok(), "commondir sain accepté");

    let cases: Vec<(&str, String)> = vec![
        ("HOME factice", home_canary.display().to_string()),
        (".ssh factice", ssh_canary.display().to_string()),
        ("relatif avec ..", "../../../../".to_string()),
        (
            "répertoire inexistant",
            base.join("inexistant").display().to_string(),
        ),
    ];
    for (label, value) in cases {
        std::fs::write(gitdir.join("commondir"), format!("{value}\n")).unwrap();
        let error = mounts
            .verify(&wt)
            .expect_err(&format!("commondir {label} doit être refusé"));
        let text = format!("{error}");
        assert!(text.contains("commondir"), "diagnostic {label} : {text}");
    }

    // Cas valide restauré : la validation repasse.
    std::fs::write(gitdir.join("commondir"), "../..\n").unwrap();
    assert!(mounts.verify(&wt).is_ok(), "commondir restauré accepté");

    // Cas lien symbolique : `commondir` qui pointe via un lien.
    let link = base.join("lien-commondir");
    std::os::unix::fs::symlink(&home_canary, &link).unwrap();
    std::fs::write(gitdir.join("commondir"), format!("{}\n", link.display())).unwrap();
    assert!(
        mounts.verify(&wt).is_err(),
        "commondir passant par un lien vers un autre répertoire est refusé"
    );

    let _ = std::fs::write(gitdir.join("commondir"), "../..\n");
    let _ = std::fs::remove_dir_all(&base);
}

/// Un dépôt dont le `.git` n'est pas un répertoire réel (lien) est refusé.
#[test]
fn un_repertoire_git_symbolique_est_refuse() {
    let base = base("symbolique");
    let (repo, _wt) = depot(&base);
    let real = common_dir(&repo);
    let link = base.join("git-allie");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let error = GitMounts::expected(&link, "wt", &base.join("wt"))
        .expect_err("répertoire Git symbolique refusé");
    assert!(format!("{error}").contains("répertoire réel"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}
