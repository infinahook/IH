//! Vérifications réelles des trois CLI dans le sandbox worktree d'IH.
//!
//! Gated par `IH_REAL_AGENTS=1` (la CI n'a ni les binaires ni les identifiants).
//! Chaque agent est lancé dans un worktree git détaché, reçoit la consigne de
//! créer un fichier, puis IH collecte les changements : le projet d'origine
//! doit rester intact (isolation) et le fichier doit apparaître comme modifié.

use ih_agent_mode::{collect_changes, create_sandbox, remove_sandbox, MAX_CHANGE_BYTES};
use ih_agents::{AgentAdapter, AgentSupervisor, StartRequest};
use ih_core::config::AgentKind;
use ih_core::events::AgentState;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

fn adapter(kind: AgentKind, id: &str, program: &str) -> AgentAdapter {
    AgentAdapter {
        id: id.into(),
        kind,
        label: id.into(),
        program: program.into(),
        args: Vec::new(),
        enabled: true,
        timeout_secs: 300,
        env: Default::default(),
        security: ih_core::config::SecurityLevel::default(),
    }
}

fn git(dir: &PathBuf, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} a échoué");
}

fn project_with_commit(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ih-real-worktree-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("projet");
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("note.txt"), "alpha\n").expect("fichier");
    git(&dir, &["add", "note.txt"]);
    git(
        &dir,
        &[
            "-c",
            "user.email=ih@test",
            "-c",
            "user.name=ih",
            "commit",
            "-qm",
            "init",
        ],
    );
    dir
}

#[test]
#[ignore = "agent réel requis : exécuter avec --ignored"]
fn worktree_isole_et_changements_reels() {
    // Agent réellement indispensable : échec explicite, jamais un succès muet.
    let cases = vec![
        adapter(AgentKind::Kilo, "kilo", "kilo"),
        adapter(AgentKind::Dsh, "dsh-cli", "dsh"),
        adapter(AgentKind::Goose, "goose", "goose"),
    ];
    for agent in cases {
        let project = project_with_commit(&agent.id);
        let base = std::env::temp_dir().join(format!("ih-sandboxes-{}", std::process::id()));
        let sandbox = create_sandbox(&project, &base, "real").expect("sandbox");
        assert_ne!(sandbox.workdir(), project.as_path());

        let supervisor =
            AgentSupervisor::new(ih_core::ResourcePolicy::default(), Arc::new(|_event| {}))
                .expect("superviseur");
        let session = supervisor
            .start(StartRequest {
                agent: agent.clone(),
                project_dir: sandbox.workdir().to_path_buf(),
                prompt: "Crée le fichier ih-real.txt contenant exactement le mot ok (sans retour à la ligne), puis réponds FAIT.".into(),
                session_id: None,

confirm_unsandboxed: false,
            })
            .expect("lancement");
        let state = supervisor
            .wait(&session, Duration::from_secs(300))
            .expect("attente");
        println!(
            "[{}] état={:?} worktree={}",
            agent.id,
            state,
            sandbox.workdir().display()
        );
        assert_eq!(
            state,
            AgentState::Completed,
            "{} : session échouée",
            agent.id
        );

        // Isolation : le projet d'origine ne contient jamais le fichier créé.
        assert!(
            !project.join("ih-real.txt").exists(),
            "{} : le projet d'origine a été modifié",
            agent.id
        );
        let changes = collect_changes(&sandbox, MAX_CHANGE_BYTES).expect("changements");
        println!(
            "[{}] fichiers modifiés={:?}",
            agent.id,
            changes.iter().map(|c| &c.path).collect::<Vec<_>>()
        );
        let created = changes
            .iter()
            .find(|change| change.path == "ih-real.txt")
            .unwrap_or_else(|| panic!("{} : fichier créé non détecté", agent.id));
        let after = created.after.clone().unwrap_or_default();
        assert!(
            after.contains("ok"),
            "{} : contenu inattendu {after:?}",
            agent.id
        );
        assert!(changes.iter().all(|c| !c.path.contains("..")));

        remove_sandbox(&sandbox).expect("nettoyage sandbox");
        let _ = std::fs::remove_dir_all(&project);
    }
}
