//! Test C — isolation Team Mode : une tâche ne doit pas pouvoir préparer une
//! évasion exploitable par la suivante via le fichier `.git` du worktree.
//!
//! Reproduit exactement l'attaque de l'audit : la session 1 empoisonne `.git`
//! depuis la sandbox, la session 2 (même worktree, comme en Team Mode) doit
//! être **refusée** et le canari de l'hôte doit rester intact.

use ih_agent_mode::{create_sandbox, remove_sandbox};
use ih_agents::sandbox::SecurityLevel;
use ih_agents::supervisor::{AgentSupervisor, StartRequest};
use ih_agents::AgentAdapter;
use ih_core::config::{AgentConfig, AgentKind};
use ih_core::ResourcePolicy;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

fn ready() -> bool {
    ih_agents::sandbox::availability().functional
}

fn require_sandbox() {
    if std::env::var("IH_REQUIRE_SANDBOX").ok().as_deref() == Some("1") && !ready() {
        panic!(
            "IH_REQUIRE_SANDBOX=1 mais la sandbox n'est pas fonctionnelle : {:?}",
            ih_agents::sandbox::availability()
        );
    }
}

fn unique(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ih-teamgit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
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

fn agent(script: &Path) -> AgentAdapter {
    AgentAdapter::from_config(&AgentConfig {
        id: "factice".into(),
        kind: AgentKind::Custom,
        label: "Factice".into(),
        program: script.display().to_string(),
        args: vec![],
        enabled: true,
        security: SecurityLevel::Isolated,
        ..Default::default()
    })
}

#[test]
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn une_tache_ne_peut_pas_preparer_une_evasion_pour_la_suivante() {
    require_sandbox();
    if !ready() {
        eprintln!("IGNORÉ: bubblewrap non fonctionnel sur cette machine");
        return;
    }

    let base = unique("run");
    let repo = base.join("depot");
    let canary = base.join("canari-hote");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&canary).unwrap();
    std::fs::write(canary.join("secret.txt"), "SECRET-HOTE-TEAM\n").unwrap();
    std::fs::write(repo.join("README.md"), "# projet\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    let sandbox = create_sandbox(&repo, &base.join("wt-base"), "team-1").expect("worktree");
    let git = sandbox.git().clone();

    // Session 1 (Lead) : l'agent empoisonne `.git` depuis la sandbox.
    let script1 = base.join("lead.sh");
    std::fs::write(
        &script1,
        format!(
            "#!/bin/sh\nprintf 'gitdir: {}\\n' > .git && echo POISON=ok\n",
            canary.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script1, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let supervisor = AgentSupervisor::new(ResourcePolicy::default(), Arc::new(|_| {})).unwrap();
    let session = supervisor
        .start(StartRequest {
            agent: agent(&script1),
            project_dir: sandbox.workdir().to_path_buf(),
            prompt: "poison".into(),
            session_id: None,
            confirm_unsandboxed: false,
            git: Some(git.clone()),
        })
        .expect("session 1 lancée");
    let state = supervisor.wait(&session, Duration::from_secs(30)).unwrap();
    assert_eq!(state, ih_core::events::AgentState::Completed);

    // Session 2 (Worker) : même worktree, montages IH inchangés → doit refuser.
    let script2 = base.join("worker.sh");
    std::fs::write(&script2, "#!/bin/sh\necho worker\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script2, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let second = supervisor.start(StartRequest {
        agent: agent(&script2),
        project_dir: sandbox.workdir().to_path_buf(),
        prompt: "suite".into(),
        session_id: None,
        confirm_unsandboxed: false,
        git: Some(git),
    });
    let error = second
        .err()
        .map(|error| error.to_string())
        .expect("la session 2 doit être refusée après empoisonnement de `.git`");
    println!("refus obtenu : {error}");
    assert!(
        error.contains("`.git`") || error.contains("modifié"),
        "diagnostic inattendu : {error}"
    );
    // Aucune trace de montage vers le canari, aucun accès réel.
    assert!(canary.join("secret.txt").exists(), "canari intact");
    assert!(!canary.join("ecrit").exists());

    remove_sandbox(&sandbox).ok();
    let _ = std::fs::remove_dir_all(&base);
}

/// Le dossier Git du worktree est hors de portée de l'agent : il ne peut pas
/// modifier `commondir` ni les métadonnées pendant une session.
#[test]
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn lagent_ne_peut_pas_ecrire_dans_le_dossier_git() {
    require_sandbox();
    if !ready() {
        eprintln!("IGNORÉ: bubblewrap non fonctionnel sur cette machine");
        return;
    }
    let base = unique("gitdir");
    let repo = base.join("depot");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# projet\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let sandbox = create_sandbox(&repo, &base.join("wt-base"), "gitdir-1").expect("worktree");
    let git = sandbox.git().clone();
    let gitdir = git.worktree_gitdir.clone();

    let script = base.join("ecrit.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho X > {}/commondir 2>&1 && echo COMMONDIR=ok || echo COMMONDIR=refused\necho X > {}/HEAD 2>&1 && echo HEAD=ok || echo HEAD=refused\n",
            gitdir.display(),
            gitdir.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let supervisor = AgentSupervisor::new(ResourcePolicy::default(), Arc::new(|_| {})).unwrap();
    let session = supervisor
        .start(StartRequest {
            agent: agent(&script),
            project_dir: sandbox.workdir().to_path_buf(),
            prompt: "test".into(),
            session_id: None,
            confirm_unsandboxed: false,
            git: Some(git),
        })
        .expect("session");
    let state = supervisor.wait(&session, Duration::from_secs(30)).unwrap();
    assert_eq!(state, ih_core::events::AgentState::Completed);
    // Le `commondir` du worktree est inchangé sur l'hôte.
    let commondir = std::fs::read_to_string(gitdir.join("commondir")).unwrap();
    assert_eq!(commondir.trim(), "../..", "commondir modifié par l'agent");

    remove_sandbox(&sandbox).ok();
    let _ = std::fs::remove_dir_all(&base);
}
