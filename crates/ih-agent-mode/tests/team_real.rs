//! Validation RÉELLE du Team Mode : plusieurs sessions d'agents dans un
//! worktree isolé, projet principal intact, diffs collectés.
//!
//! Ignoré par défaut (réseau + CLI d'agents requis). Exécution :
//!   cargo test -p ih-agent-mode --test team_real -- --ignored --nocapture

use ih_agent_mode::{
    collect_changes, create_sandbox, remove_sandbox, run_team_tasks, MAX_CHANGE_BYTES,
};
use ih_agents::supervisor::{AgentSupervisor, EventSink};
use ih_core::config::AppConfig;
use ih_core::ResourcePolicy;
use ih_orchestrator::{plan_goal, Team, TeamRole};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
#[ignore = "exécution réelle : CLI d'agents et réseau requis"]
fn equipe_reelle_travaille_dans_le_worktree() {
    let base = std::env::temp_dir().join(format!("ih-team-real-{}", std::process::id()));
    let project = base.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("README.md"), "# Projet de validation\n").unwrap();
    std::fs::write(
        project.join("src/lib.rs"),
        "pub fn addition(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    std::fs::write(project.join("NOTES.md"), "# Notes\n\n- initial\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@ih"],
        vec!["config", "user.name", "IH"],
        vec!["add", "."],
        vec!["commit", "-qm", "initial"],
    ] {
        git(&project, &args);
    }
    let before = git(&project, &["status", "--porcelain"]);
    assert!(before.is_empty(), "dépôt initial propre");

    let config = AppConfig::default();
    let team = Team::new("reelle")
        .with_member("opencode", TeamRole::Lead)
        .with_member("pi", TeamRole::Worker)
        .with_member("opencode", TeamRole::Worker)
        .with_member("pi", TeamRole::Reviewer);
    team.validate().unwrap();

    let goal = "Dans ce dépôt : ajoute à la fin de src/lib.rs la fonction \
                `pub fn soustraction(a: i32, b: i32) -> i32 { a - b }` et ajoute \
                une ligne '- validation equipe' à la fin de NOTES.md. \
                Ne touche à aucun autre fichier.";
    let plan = plan_goal(goal, &team, 2).unwrap();
    plan.graph.validate().unwrap();
    assert!(plan.graph.iter().count() >= 3, "plan à plusieurs tâches");

    let sink: EventSink = Arc::new(|event| println!("événement: {event:?}"));
    let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
    let sandbox = create_sandbox(&project, &base.join("worktrees"), "run-reelle").unwrap();
    assert!(sandbox.workdir().starts_with(base.join("worktrees")));

    let outcomes = run_team_tasks(
        &supervisor,
        &sandbox,
        &config,
        &plan,
        goal,
        Duration::from_secs(300),
    )
    .unwrap();

    for outcome in &outcomes {
        println!(
            "tâche {} | rôle {} | agent {} | état {}",
            outcome.title, outcome.role, outcome.agent_id, outcome.state
        );
    }
    let completed = outcomes.iter().filter(|o| o.state == "completed").count();
    assert_eq!(
        completed,
        outcomes.len(),
        "toutes les tâches doivent aboutir"
    );
    assert!(outcomes.iter().any(|o| o.agent_id == "pi"), "pi a exécuté");
    assert!(
        outcomes.iter().any(|o| o.agent_id == "opencode"),
        "OpenCode a exécuté"
    );

    let changes = collect_changes(&sandbox, MAX_CHANGE_BYTES).unwrap();
    println!(
        "fichiers modifiés : {:?}",
        changes.iter().map(|c| &c.path).collect::<Vec<_>>()
    );
    assert!(
        !changes.is_empty(),
        "l'équipe doit produire des modifications"
    );

    // --- Revue des diffs : accepter un fichier, rejeter l'autre ---
    use ih_workspace::fs::ProjectFs;
    let fs = ProjectFs::new(&project).unwrap();
    let lib = changes
        .iter()
        .find(|c| c.path == "src/lib.rs")
        .expect("src/lib.rs modifié par l'équipe");
    let before_sha = fs.metadata("src/lib.rs").unwrap().sha256;
    let backup_dir = base.join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let outcome = fs
        .write_checked(
            "src/lib.rs",
            lib.after.as_deref().expect("contenu produit"),
            Some(&before_sha),
            Some(backup_dir.as_path()),
        )
        .expect("acceptation du diff");
    println!(
        "accepté : {:?} sauvegarde={:?}",
        outcome.bytes, outcome.backup
    );
    assert!(outcome.backup.is_some(), "sauvegarde créée avant écriture");
    let written = std::fs::read_to_string(project.join("src/lib.rs")).unwrap();
    assert!(
        written.contains("soustraction"),
        "contenu réellement écrit : {written}"
    );
    assert!(
        written.contains("addition"),
        "le reste du fichier est préservé"
    );

    // Rejet : on n'applique pas NOTES.md, le fichier doit rester intact.
    let notes_before = std::fs::read_to_string(project.join("NOTES.md")).unwrap();
    let notes_change = changes.iter().find(|c| c.path == "NOTES.md").unwrap();
    assert_ne!(
        notes_change.after.as_deref().unwrap_or_default(),
        notes_before,
        "l'agent a bien proposé un changement pour NOTES.md"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("NOTES.md")).unwrap(),
        notes_before,
        "un diff rejeté n'est pas appliqué"
    );

    let after = git(&project, &["status", "--porcelain"]);
    assert_eq!(
        after.trim(),
        "M src/lib.rs",
        "seul le diff accepté est écrit dans le dépôt : {after}"
    );

    remove_sandbox(&sandbox).unwrap();
    assert!(!sandbox.workdir().exists(), "worktree supprimé");
    std::fs::remove_dir_all(&base).ok();
}
