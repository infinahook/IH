//! Flux d'événements du Mode Équipe : ordre, états, absence de doublon.
//!
//! Utilise un agent factice (script) : pas de réseau ni de LLM, mais le vrai
//! chemin `run_team_tasks_with_events` → collecte des événements.

use ih_agent_mode::{create_sandbox, remove_sandbox, run_team_tasks_with_events, TeamTaskOutcome};
use ih_agents::supervisor::{AgentSupervisor, EventSink};
use ih_core::config::{AgentConfig, AgentKind, AppConfig, SecurityLevel};
use ih_core::ResourcePolicy;
use ih_orchestrator::{plan_goal, Team, TeamRole};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

fn fake_agent(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn evenements_ordonnes_et_sans_doublon() {
    let base = std::env::temp_dir().join(format!("ih-team-events-{}", std::process::id()));
    let project = base.join("projet");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("README.md"), "# t\n").unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["config", "user.email", "t@ih"]);
    git(&project, &["config", "user.name", "IH"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "init"]);

    let script = fake_agent(
        &base,
        "agent-factice",
        "#!/bin/sh\necho \"travail fait\"\nexit 0\n",
    );
    let mut config = AppConfig::default();
    config.agents.push(AgentConfig {
        id: "factice".into(),
        kind: AgentKind::Custom,
        label: "Factice".into(),
        program: script.display().to_string(),
        enabled: true,
        // Test du flux d'événements, pas de la sandbox : agent factice exécuté
        // sans isolation (la confirmation est donnée par le mode Équipe).
        security: SecurityLevel::None,
        ..Default::default()
    });

    let team = Team::new("t")
        .with_member("factice", TeamRole::Lead)
        .with_member("factice", TeamRole::Worker)
        .with_member("factice", TeamRole::Reviewer);
    let plan = plan_goal("objectif de test", &team, 2).unwrap();

    let sink: EventSink = Arc::new(|_| {});
    let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
    let sandbox = create_sandbox(&project, &base.join("wt"), "evts").unwrap();

    let mut events: Vec<TeamTaskOutcome> = Vec::new();
    let outcomes = run_team_tasks_with_events(
        &supervisor,
        &sandbox,
        &config,
        &plan,
        "objectif de test",
        Duration::from_secs(20),
        &mut |outcome| events.push(outcome.clone()),
    )
    .unwrap();

    // Deux événements par tâche : running puis état final, dans cet ordre.
    assert_eq!(
        events.len(),
        outcomes.len() * 2,
        "un running et un final par tâche"
    );
    for (index, outcome) in outcomes.iter().enumerate() {
        let running = &events[index * 2];
        let finished = &events[index * 2 + 1];
        assert_eq!(running.task_id, outcome.task_id);
        assert_eq!(running.state, "running");
        assert_eq!(finished.task_id, outcome.task_id);
        assert_eq!(finished.state, outcome.state);
        assert_eq!(finished.role, outcome.role);
    }
    // Ordre d'exécution : lead d'abord, reviewer en dernier.
    assert_eq!(events[0].role, "lead");
    assert_eq!(events[events.len() - 1].role, "reviewer");
    // Aucune tâche ne reçoit deux états finaux différents.
    let mut finals: Vec<&str> = events
        .iter()
        .filter(|e| e.state != "running")
        .map(|e| e.state.as_str())
        .collect();
    assert!(finals.iter().all(|s| *s == "completed"), "{finals:?}");
    finals.dedup();

    remove_sandbox(&sandbox).unwrap();
    std::fs::remove_dir_all(&base).ok();
}
