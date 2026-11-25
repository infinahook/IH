//! Tests réels contre les CLI d'agents installés (Kilo, DSH CLI, Goose).
//!
//! Ces tests ne s'exécutent **que** si `IH_REAL_AGENTS=1` : la CI n'a ni les
//! binaires ni les identifiants. Ils lancent réellement chaque agent dans un
//! répertoire temporaire, avec une consigne minimale, et vérifient le contrat
//! d'intégration : détection, version, lancement, réponse, annulation et erreur.
//!
//! Aucun résultat n'est simulé : si un CLI ne répond pas, le test échoue.

use ih_agents::{AgentAdapter, AgentSupervisor, StartRequest};
use ih_core::config::AgentKind;
use ih_core::events::{AgentState, CoreEvent};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn enabled() -> bool {
    std::env::var("IH_REAL_AGENTS").ok().as_deref() == Some("1")
}

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

fn agents() -> Vec<AgentAdapter> {
    vec![
        adapter(AgentKind::Kilo, "kilo", "kilo"),
        adapter(AgentKind::Dsh, "dsh-cli", "dsh"),
        adapter(AgentKind::Goose, "goose", "goose"),
    ]
}

fn temp_workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ih-real-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dossier temporaire");
    dir
}

#[test]
fn detection_et_version_reelles() {
    if !enabled() {
        return;
    }
    for agent in agents() {
        let path = agent
            .detection()
            .unwrap_or_else(|| panic!("{} introuvable", agent.id));
        println!("[{}] résolu -> {}", agent.id, path.display());
        assert!(path.is_file(), "{} : chemin non exécutable", agent.id);
        let version = ih_agents::adapters::probe_version(&path)
            .unwrap_or_else(|| panic!("{} : version illisible", agent.id));
        println!(
            "[{}] chemin={} version={}",
            agent.id,
            path.display(),
            version
        );
        assert!(
            version.contains('.'),
            "{} : version inattendue {version}",
            agent.id
        );
    }
}

#[test]
fn lancement_instruction_et_reponse_reelles() {
    if !enabled() {
        return;
    }
    for agent in agents() {
        let dir = temp_workdir(&format!("run-{}", agent.id));
        let events: Arc<Mutex<Vec<CoreEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_events = events.clone();
        let supervisor = AgentSupervisor::new(
            ih_core::ResourcePolicy::default(),
            Arc::new(move |event| sink_events.lock().unwrap().push(event)),
        )
        .expect("superviseur");
        let session = supervisor
            .start(StartRequest {
                agent: agent.clone(),
                project_dir: dir.clone(),
                prompt:
                    "Réponds uniquement par le mot OK, sans appeler d'outil ni créer de fichier."
                        .into(),
                session_id: None,
                confirm_unsandboxed: false,
            })
            .expect("lancement");
        let state = supervisor
            .wait(&session, Duration::from_secs(300))
            .expect("attente");
        let stderr: String = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                CoreEvent::AgentOutput { stream, data, .. }
                    if *stream == ih_core::events::OutputStream::Stderr =>
                {
                    Some(data.clone())
                }
                _ => None,
            })
            .collect();
        println!(
            "[{}] stderr={:?} détection={:?} env={:?}",
            agent.id,
            stderr,
            agent.detection(),
            std::env::var("DSH_HOME")
        );
        let stdout: String = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                CoreEvent::AgentOutput { stream, data, .. }
                    if *stream == ih_core::events::OutputStream::Stdout =>
                {
                    Some(data.clone())
                }
                _ => None,
            })
            .collect();
        println!("[{}] état={:?} sortie={:?}", agent.id, state, stdout.trim());
        assert_eq!(
            state,
            AgentState::Completed,
            "{} : échec du lancement",
            agent.id
        );
        assert!(
            stdout.contains("OK"),
            "{} : réponse attendue absente de la sortie normalisée",
            agent.id
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn annulation_reelle_tue_le_processus() {
    if !enabled() {
        return;
    }
    for agent in agents() {
        let dir = temp_workdir(&format!("cancel-{}", agent.id));
        let supervisor =
            AgentSupervisor::new(ih_core::ResourcePolicy::default(), Arc::new(|_event| {}))
                .expect("superviseur");
        let mut slow = agent.clone();
        slow.timeout_secs = 600;
        let session = supervisor
            .start(StartRequest {
                agent: slow,
                project_dir: dir.clone(),
                prompt: "Compte lentement de 1 à 50 en détaillant chaque nombre.".into(),
                session_id: None,
                confirm_unsandboxed: false,
            })
            .expect("lancement");
        // Laisse le CLI démarrer réellement, puis annule.
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(2500) {
            std::thread::sleep(Duration::from_millis(200));
        }
        supervisor.cancel(&session).expect("annulation");
        let state = supervisor
            .wait(&session, Duration::from_secs(30))
            .expect("attente après annulation");
        println!("[{}] état après annulation={:?}", agent.id, state);
        assert_eq!(
            state,
            AgentState::Cancelled,
            "{} : annulation non effective",
            agent.id
        );
        // Le groupe de processus doit avoir disparu (aucun enfant orphelin).
        // Le processus peut être un zombie quelques millisecondes le temps
        // d'être récolté par le superviseur : on tolère cet état transitoire.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let Some(pid) = supervisor
                .snapshot(&session)
                .ok()
                .flatten()
                .and_then(|snapshot| snapshot.pid)
            else {
                break;
            };
            let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
            let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|stat| stat.split_whitespace().nth(2) == Some("Z"))
                .unwrap_or(true);
            if !alive || zombie {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                Instant::now() < deadline,
                "{} : processus encore vivant après annulation ({pid})",
                agent.id
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn erreur_reelle_est_signalee() {
    if !enabled() {
        return;
    }
    let cases = vec![
        (
            adapter(AgentKind::Kilo, "kilo", "kilo"),
            "modeles-test-absents",
            "inexistant/xyz",
        ),
        (
            adapter(AgentKind::Goose, "goose", "goose"),
            "provider",
            "inexistant",
        ),
        (
            adapter(AgentKind::Dsh, "dsh-cli", "dsh"),
            "profil",
            "absent",
        ),
    ];
    for (mut agent, mode, value) in cases {
        let dir = temp_workdir(&format!("error-{}", agent.id));
        let _ = mode;
        // Injection d'une configuration volontairement invalide, sans secret.
        match agent.kind {
            AgentKind::Kilo => {
                agent.args = vec![
                    "run".into(),
                    "--format".into(),
                    "json".into(),
                    "--model".into(),
                    value.into(),
                ];
            }
            AgentKind::Goose => {
                agent.env.insert("GOOSE_PROVIDER".into(), value.into());
                agent.env.insert("GOOSE_MODEL".into(), value.into());
            }
            AgentKind::Dsh => {
                agent.args = vec!["--profile".into(), value.into()];
            }
            _ => {}
        }
        let supervisor =
            AgentSupervisor::new(ih_core::ResourcePolicy::default(), Arc::new(|_event| {}))
                .expect("superviseur");
        let session = supervisor
            .start(StartRequest {
                agent: agent.clone(),
                project_dir: dir.clone(),
                prompt: "OK".into(),
                session_id: None,
                confirm_unsandboxed: false,
            })
            .expect("lancement");
        let state = supervisor
            .wait(&session, Duration::from_secs(120))
            .expect("attente");
        println!("[{}] état d'erreur={:?}", agent.id, state);
        assert_eq!(
            state,
            AgentState::Failed,
            "{} : une configuration invalide doit échouer",
            agent.id
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
