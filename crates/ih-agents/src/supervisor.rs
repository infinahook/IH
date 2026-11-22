//! Supervision des processus d'agents.
//!
//! Un processus par session, sortie bornée, événements normalisés, annulation
//! et expiration garanties. La limite de sessions simultanées vient de
//! `ResourcePolicy` et n'est pas contournable par l'interface.

use crate::adapters::AgentAdapter;
use ih_core::events::{AgentState, CoreEvent, OutputStream};
use ih_core::{IhError, IhResult, ResourcePolicy, SessionId};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Callback d'émission d'événements (l'interface Tauri y branche `app.emit`).
pub type EventSink = Arc<dyn Fn(CoreEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct StartRequest {
    pub agent: AgentAdapter,
    pub project_dir: PathBuf,
    pub prompt: String,
    /// Identifiant de session fourni par l'appelant (persisté en base).
    pub session_id: Option<SessionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub pid: Option<u32>,
    pub agent_id: String,
    pub state: AgentState,
    pub exit_code: Option<i32>,
    pub bytes_emitted: u64,
}

struct Running {
    pid: Option<u32>,
    state: Mutex<AgentState>,
    child: Mutex<Option<std::process::Child>>,
    cancelled: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    exit_code: Mutex<Option<i32>>,
    agent_id: String,
}

/// État final conservé après la disparition de la session active, pour que
/// `wait`/`snapshot` ne dépendent pas d'une course avec le thread de veille.
#[derive(Debug, Clone)]
struct Finished {
    state: AgentState,
    exit_code: Option<i32>,
    agent_id: String,
}

const FINISHED_CAPACITY: usize = 128;

pub struct AgentSupervisor {
    policy: ResourcePolicy,
    sink: EventSink,
    active: Arc<AtomicUsize>,
    sessions: Arc<Mutex<HashMap<String, Arc<Running>>>>,
    finished: Arc<Mutex<HashMap<String, Finished>>>,
}

impl AgentSupervisor {
    pub fn new(policy: ResourcePolicy, sink: EventSink) -> IhResult<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            sink,
            active: Arc::new(AtomicUsize::new(0)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            finished: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn policy(&self) -> &ResourcePolicy {
        &self.policy
    }

    pub fn active_count(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    pub fn snapshot(&self, session_id: &str) -> IhResult<Option<SessionSnapshot>> {
        let guard = self
            .sessions
            .lock()
            .map_err(|_| IhError::Internal("registre de sessions empoisonné".into()))?;
        let Some(running) = guard.get(session_id) else {
            drop(guard);
            let finished = self
                .finished
                .lock()
                .map_err(|_| IhError::Internal("historique empoisonné".into()))?;
            return Ok(finished.get(session_id).map(|f| SessionSnapshot {
                session_id: session_id.to_string(),
                pid: None,
                agent_id: f.agent_id.clone(),
                state: f.state,
                exit_code: f.exit_code,
                bytes_emitted: 0,
            }));
        };
        let state = *running
            .state
            .lock()
            .map_err(|_| IhError::Internal("état de session empoisonné".into()))?;
        let exit_code = *running
            .exit_code
            .lock()
            .map_err(|_| IhError::Internal("code de sortie empoisonné".into()))?;
        Ok(Some(SessionSnapshot {
            session_id: session_id.to_string(),
            pid: running.pid,
            agent_id: running.agent_id.clone(),
            state,
            exit_code,
            bytes_emitted: running.bytes.load(Ordering::SeqCst),
        }))
    }

    /// Démarre une session. La limite `max_concurrent_agents` est vérifiée
    /// avant tout lancement de processus.
    pub fn start(&self, request: StartRequest) -> IhResult<String> {
        if !request.agent.enabled {
            return Err(IhError::Agent(format!(
                "l'agent {} est désactivé",
                request.agent.id
            )));
        }
        if request.agent.detection().is_none() {
            return Err(IhError::Agent(format!(
                "programme introuvable dans le PATH : {}",
                request.agent.program
            )));
        }
        if request.prompt.trim().is_empty() {
            return Err(IhError::Invalid("instruction vide".into()));
        }
        if !request.project_dir.is_dir() {
            return Err(IhError::Invalid(format!(
                "dossier de projet inexistant : {}",
                request.project_dir.display()
            )));
        }
        if self.active.load(Ordering::SeqCst) >= self.policy.max_concurrent_agents as usize {
            return Err(IhError::Limit(format!(
                "maximum de {} sessions d'agent simultanées atteint",
                self.policy.max_concurrent_agents
            )));
        }

        let session_id = request.session_id.clone().unwrap_or_default().into_string();
        let (program, args) = request.agent.build_command(&request.prompt);

        let mut command = Command::new(&program);
        command
            .args(&args)
            .current_dir(&request.project_dir)
            // Certains CLI (OpenCode v2.3.2 vérifié) résolvent leur projet via
            // `$PWD` et ignorent le répertoire courant réel : sans cela, ils
            // écrivent dans le répertoire d'où IH a été lancé.
            .env("PWD", &request.project_dir)
            .env("OLDPWD", &request.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("IH_SESSION_ID", &session_id);

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Groupe de processus propre : l'annulation tue aussi les enfants.
            command.process_group(0);
        }

        // `ETXTBSY` peut survenir si un script vient d'être écrit (descripteur
        // encore ouvert par un processus concurrent) : nouvelle tentative brève.
        let mut attempts = 0;
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(e) if e.raw_os_error() == Some(26) && attempts < 20 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(IhError::Agent(format!(
                        "lancement de {} impossible : {e}",
                        request.agent.program
                    )))
                }
            }
        };

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| IhError::Internal("stdout indisponible".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| IhError::Internal("stderr indisponible".into()))?;
        let pid = child.id();

        let cancelled = Arc::new(AtomicBool::new(false));
        let bytes = Arc::new(AtomicU64::new(0));
        let running = Arc::new(Running {
            pid: Some(pid),
            state: Mutex::new(AgentState::Running),
            child: Mutex::new(Some(child)),
            cancelled: cancelled.clone(),
            bytes: bytes.clone(),
            exit_code: Mutex::new(None),
            agent_id: request.agent.id.clone(),
        });

        {
            let mut guard = self
                .sessions
                .lock()
                .map_err(|_| IhError::Internal("registre de sessions empoisonné".into()))?;
            guard.insert(session_id.clone(), running.clone());
        }
        self.active.fetch_add(1, Ordering::SeqCst);

        self.emit_state(&session_id, AgentState::Running);

        let max_output = self.policy.max_output_bytes_per_session;
        let agent_kind = request.agent.kind.clone();
        let sink = self.sink.clone();
        let seq_out = Arc::new(AtomicU64::new(0));
        let seq_err = seq_out.clone();
        let bytes_out = bytes.clone();
        let bytes_err = bytes.clone();
        let sid_out = session_id.clone();
        let sid_err = session_id.clone();

        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            pump(
                reader,
                OutputStream::Stdout,
                &sid_out,
                max_output,
                bytes_out,
                sink,
                seq_out,
                Some(agent_kind),
            )
        });
        let sink_err = self.sink.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            pump(
                reader,
                OutputStream::Stderr,
                &sid_err,
                max_output,
                bytes_err,
                sink_err,
                seq_err,
                None,
            )
        });

        // Thread de surveillance : expiration, annulation, collecte du code de sortie.
        let watchdog_sink = self.sink.clone();
        let watchdog_states = self.sessions.clone();
        let watchdog_finished = self.finished.clone();
        let watchdog_session = session_id.clone();
        let watchdog_running = running.clone();
        let timeout = Duration::from_secs(request.agent.timeout_secs.max(1));
        let active = self.active.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let mut final_state = AgentState::Completed;
            let mut code: Option<i32> = None;

            loop {
                if cancelled.load(Ordering::SeqCst) {
                    final_state = AgentState::Cancelled;
                    break;
                }
                if started.elapsed() > timeout {
                    final_state = AgentState::Failed;
                    break;
                }
                let status = {
                    let Ok(mut guard) = watchdog_running.child.lock() else {
                        break;
                    };
                    let Some(child) = guard.as_mut() else {
                        break;
                    };
                    match child.try_wait() {
                        Ok(Some(status)) => Some(status),
                        Ok(None) => None,
                        Err(_) => {
                            final_state = AgentState::Failed;
                            break;
                        }
                    }
                };
                if let Some(status) = status {
                    code = status.code();
                    if !status.success() {
                        final_state = AgentState::Failed;
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            // Termine le groupe de processus (enfants compris) dans tous les cas.
            terminate(pid);
            // Récolte le processus enfant : sans `wait`, il resterait zombie
            // jusqu'à la fermeture d'IH.
            if let Ok(mut guard) = watchdog_running.child.lock() {
                if let Some(child) = guard.as_mut() {
                    let _ = child.wait();
                }
            }

            if let Ok(mut guard) = watchdog_running.state.lock() {
                *guard = final_state;
            }
            if let Ok(mut guard) = watchdog_running.exit_code.lock() {
                *guard = code;
            }
            watchdog_sink(CoreEvent::AgentState {
                session_id: watchdog_session.clone(),
                state: final_state,
            });
            if let Ok(mut guard) = watchdog_finished.lock() {
                if guard.len() >= FINISHED_CAPACITY {
                    if let Some(key) = guard.keys().next().cloned() {
                        guard.remove(&key);
                    }
                }
                guard.insert(
                    watchdog_session.clone(),
                    Finished {
                        state: final_state,
                        exit_code: code,
                        agent_id: watchdog_running.agent_id.clone(),
                    },
                );
            }
            if let Ok(mut guard) = watchdog_states.lock() {
                guard.remove(&watchdog_session);
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });

        Ok(session_id)
    }

    /// Demande l'arrêt d'une session (le groupe de processus est tué).
    pub fn cancel(&self, session_id: &str) -> IhResult<()> {
        let guard = self
            .sessions
            .lock()
            .map_err(|_| IhError::Internal("registre de sessions empoisonné".into()))?;
        let running = guard
            .get(session_id)
            .ok_or_else(|| IhError::NotFound(format!("session {session_id}")))?;
        running.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut child) = running.child.lock() {
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
        if let Ok(mut state) = running.state.lock() {
            if !state.is_terminal() {
                *state = AgentState::Cancelled;
            }
        }
        Ok(())
    }

    /// Attend la fin d'une session (utile pour les tests et l'orchestrateur).
    pub fn wait(&self, session_id: &str, timeout: Duration) -> IhResult<AgentState> {
        let deadline = Instant::now() + timeout;
        loop {
            let state = {
                let guard = self
                    .sessions
                    .lock()
                    .map_err(|_| IhError::Internal("registre empoisonné".into()))?;
                match guard.get(session_id) {
                    Some(running) => Some(
                        *running
                            .state
                            .lock()
                            .map_err(|_| IhError::Internal("état empoisonné".into()))?,
                    ),
                    None => None,
                }
            };
            match state {
                Some(AgentState::Completed) => return Ok(AgentState::Completed),
                Some(AgentState::Failed) => return Ok(AgentState::Failed),
                Some(AgentState::Cancelled) => return Ok(AgentState::Cancelled),
                Some(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Some(other) => return Ok(other),
                None => {
                    let finished = self
                        .finished
                        .lock()
                        .map_err(|_| IhError::Internal("historique empoisonné".into()))?;
                    return match finished.get(session_id) {
                        Some(entry) => Ok(entry.state),
                        None => Err(IhError::NotFound(format!("session {session_id}"))),
                    };
                }
            }
        }
    }

    fn emit_state(&self, session_id: &str, state: AgentState) {
        (self.sink)(CoreEvent::AgentState {
            session_id: session_id.to_string(),
            state,
        });
    }
}

/// Lit un flux ligne à ligne et émet des événements bornés.
// Boucle de lecture interne : les paramètres sont stables et explicites,
// les regrouper dans une structure n'apporterait rien.
#[allow(clippy::too_many_arguments)]
fn pump<R: Read>(
    reader: BufReader<R>,
    stream: OutputStream,
    session_id: &str,
    max_bytes: u64,
    counter: Arc<AtomicU64>,
    sink: EventSink,
    seq: Arc<AtomicU64>,
    kind: Option<ih_core::config::AgentKind>,
) {
    let mut truncated_notified = false;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        // La sortie structurée (OpenCode JSON) est convertie en lignes
        // lisibles ; les autres adaptateurs passent inchangés.
        let rendered = match &kind {
            Some(kind) => crate::normalize::normalize_line(kind, &line),
            None => vec![line.clone()],
        };
        if rendered.is_empty() {
            continue;
        }
        let chunk = format!("{}\n", rendered.join("\n"));
        let len = chunk.len() as u64;
        let previous = counter.fetch_add(len, Ordering::SeqCst);
        if previous + len > max_bytes {
            if !truncated_notified {
                truncated_notified = true;
                sink(CoreEvent::AgentOutput {
                    session_id: session_id.to_string(),
                    stream,
                    seq: seq.fetch_add(1, Ordering::SeqCst),
                    data: format!("[sortie tronquée : limite de {max_bytes} octets atteinte]\n"),
                });
            }
            continue;
        }
        sink(CoreEvent::AgentOutput {
            session_id: session_id.to_string(),
            stream,
            seq: seq.fetch_add(1, Ordering::SeqCst),
            data: chunk,
        });
    }
}

#[cfg(unix)]
fn terminate(pid: u32) {
    // SIGKILL sur le groupe de processus créé avec `process_group(0)`.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate(_pid: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ih_core::config::AgentKind;
    use std::sync::Mutex as StdMutex;

    fn collector() -> (EventSink, Arc<StdMutex<Vec<CoreEvent>>>) {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let sink: EventSink = {
            let events = events.clone();
            Arc::new(move |event| events.lock().unwrap().push(event))
        };
        (sink, events)
    }

    /// Écrit le script dans un fichier temporaire puis le renomme : exécuter
    /// directement un fichier encore ouvert en écriture provoque `ETXTBSY`.
    fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        let staging = dir.join(format!(".{name}.tmp"));
        std::fs::write(&staging, body).unwrap();
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&staging, &path).unwrap();
        path
    }

    fn adapter_for(path: &std::path::Path) -> AgentAdapter {
        AgentAdapter {
            id: "faux".into(),
            kind: AgentKind::Custom,
            label: "Faux agent".into(),
            program: path.display().to_string(),
            args: vec![],
            enabled: true,
            timeout_secs: 5,
        }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ih-agents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn session_reussie_emet_la_sortie_et_l_etat_final() {
        let dir = temp_dir();
        let prog = script(&dir, "ok.sh", "#!/bin/sh\necho bonjour\necho fin\n");
        let (sink, events) = collector();
        let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
        let id = supervisor
            .start(StartRequest {
                agent: adapter_for(&prog),
                project_dir: dir.clone(),
                prompt: "salut".to_string(),
                session_id: None,
            })
            .unwrap();
        let state = supervisor.wait(&id, Duration::from_secs(5)).unwrap();
        assert_eq!(state, AgentState::Completed);

        let collected = events.lock().unwrap().clone();
        let text: String = collected
            .iter()
            .filter_map(|e| match e {
                CoreEvent::AgentOutput { data, .. } => Some(data.clone()),
                _ => None,
            })
            .collect();
        assert!(text.contains("bonjour"), "sortie: {text}");
        assert!(text.contains("fin"));
        assert!(collected.iter().any(|e| matches!(
            e,
            CoreEvent::AgentState {
                state: AgentState::Completed,
                ..
            }
        )));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn code_de_sortie_non_nul_donne_echec() {
        let dir = temp_dir();
        let prog = script(&dir, "fail.sh", "#!/bin/sh\necho erreur >&2\nexit 3\n");
        let (sink, events) = collector();
        let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
        let id = supervisor
            .start(StartRequest {
                agent: adapter_for(&prog),
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap();
        assert_eq!(
            supervisor.wait(&id, Duration::from_secs(5)).unwrap(),
            AgentState::Failed
        );
        let collected = events.lock().unwrap().clone();
        assert!(collected.iter().any(|e| matches!(
            e,
            CoreEvent::AgentOutput {
                stream: OutputStream::Stderr,
                ..
            }
        )));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn annulation_tue_la_session() {
        let dir = temp_dir();
        let prog = script(&dir, "sleep.sh", "#!/bin/sh\necho debut\nsleep 30\n");
        let (sink, _events) = collector();
        let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
        let id = supervisor
            .start(StartRequest {
                agent: adapter_for(&prog),
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let pid = supervisor.snapshot(&id).ok().flatten().and_then(|s| s.pid);
        supervisor.cancel(&id).unwrap();
        assert_eq!(
            supervisor.wait(&id, Duration::from_secs(5)).unwrap(),
            AgentState::Cancelled
        );
        // Le processus doit être mort ET récolté (aucun zombie).
        if let Some(pid) = pid {
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "le processus {pid} doit être récolté"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn le_processus_recoit_pwd_egal_a_son_repertoire_de_travail() {
        let dir = temp_dir();
        let out = dir.join("pwd.txt");
        let prog = script(
            &dir,
            "pwd.sh",
            &format!("#!/bin/sh\nprintf '%s' \"$PWD\" > {}\n", out.display()),
        );
        let (sink, _events) = collector();
        let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
        let id = supervisor
            .start(StartRequest {
                agent: adapter_for(&prog),
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap();
        let _ = supervisor.wait(&id, Duration::from_secs(5));
        let observed = std::fs::read_to_string(&out).unwrap();
        assert_eq!(
            std::path::Path::new(&observed),
            dir.canonicalize().unwrap(),
            "le processus doit voir PWD = son répertoire de travail"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn limite_de_sessions_simultanees_respectee() {
        let dir = temp_dir();
        let prog = script(&dir, "long.sh", "#!/bin/sh\nsleep 20\n");
        let (sink, _events) = collector();
        let policy = ResourcePolicy {
            max_concurrent_agents: 1,
            ..Default::default()
        };
        let supervisor = AgentSupervisor::new(policy, sink).unwrap();
        let first = supervisor
            .start(StartRequest {
                agent: adapter_for(&prog),
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap();
        let second = supervisor.start(StartRequest {
            agent: adapter_for(&prog),
            project_dir: dir.clone(),
            prompt: "y".to_string(),
            session_id: None,
        });
        assert!(matches!(second, Err(IhError::Limit(_))));
        supervisor.cancel(&first).ok();
        let _ = supervisor.wait(&first, Duration::from_secs(5));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn agent_absent_ou_instruction_vide_refuses() {
        let dir = temp_dir();
        let (sink, _events) = collector();
        let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
        let mut adapter = adapter_for(&dir.join("absent.sh"));
        let err = supervisor
            .start(StartRequest {
                agent: adapter.clone(),
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("introuvable"));

        adapter.program = "sh".into();
        let err = supervisor
            .start(StartRequest {
                agent: adapter,
                project_dir: dir.clone(),
                prompt: "   ".to_string(),
                session_id: None,
            })
            .unwrap_err();
        assert!(matches!(err, IhError::Invalid(_)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sortie_bornee_a_la_limite_configuree() {
        let dir = temp_dir();
        let prog = script(
            &dir,
            "bavard.sh",
            "#!/bin/sh\ni=0\nwhile [ $i -lt 200 ]; do echo \"ligne numero $i sur plusieurs octets\"; i=$((i+1)); done\n",
        );
        let (sink, events) = collector();
        let policy = ResourcePolicy {
            max_output_bytes_per_session: 256,
            ..Default::default()
        };
        let supervisor = AgentSupervisor::new(policy, sink).unwrap();
        let id = supervisor
            .start(StartRequest {
                agent: adapter_for(&prog),
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap();
        let _ = supervisor.wait(&id, Duration::from_secs(10));
        let collected = events.lock().unwrap().clone();
        let total: usize = collected
            .iter()
            .filter_map(|e| match e {
                CoreEvent::AgentOutput { data, .. } => Some(data.len()),
                _ => None,
            })
            .sum();
        assert!(total < 4096, "sortie non bornée : {total} octets");
        assert!(collected.iter().any(|e| matches!(
            e,
            CoreEvent::AgentOutput { data, .. } if data.contains("tronquée")
        )));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn timeout_marque_echec() {
        let dir = temp_dir();
        let prog = script(&dir, "lent.sh", "#!/bin/sh\nsleep 30\n");
        let (sink, _events) = collector();
        let supervisor = AgentSupervisor::new(ResourcePolicy::default(), sink).unwrap();
        let mut adapter = adapter_for(&prog);
        adapter.timeout_secs = 1;
        let id = supervisor
            .start(StartRequest {
                agent: adapter,
                project_dir: dir.clone(),
                prompt: "x".to_string(),
                session_id: None,
            })
            .unwrap();
        assert_eq!(
            supervisor.wait(&id, Duration::from_secs(8)).unwrap(),
            AgentState::Failed
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
