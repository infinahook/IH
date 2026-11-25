//! Vérifications réelles de l'isolation système (bubblewrap).
//!
//! Ces tests ne supposent rien : si bubblewrap n'est pas opérationnel sur la
//! machine (CI sans paquet, user namespaces restreints), ils s'arrêtent
//! immédiatement sans échouer. Sur une machine équipée, ils prouvent par des
//! canaris que la sandbox bloque réellement l'accès au reste du système.

use ih_agents::sandbox::{self, SecurityLevel};
use ih_core::config::AgentKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ready() -> bool {
    sandbox::availability().functional
}

fn unique(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ih-sbx-{tag}-{}", std::process::id()))
}

/// Exécute une commande shell dans la sandbox et renvoie (code, stdout, stderr).
fn run_in_sandbox(
    level: SecurityLevel,
    workdir: &Path,
    script: &str,
    env: &[(String, String)],
) -> (bool, String, String) {
    let plan = sandbox::sandboxed_command(
        level,
        &AgentKind::Custom,
        Path::new("/bin/sh"),
        &["-c".to_string(), script.to_string()],
        workdir,
        env,
        "test",
    )
    .expect("plan de sandbox");
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

fn wait_port(port: u16) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("serveur de test injoignable");
}

#[test]
fn la_sandbox_bloque_les_acces_hors_worktree() {
    if !ready() {
        return;
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let canary_dir = home.join(".cache");
    let _ = std::fs::create_dir_all(&canary_dir);
    let canary = canary_dir.join(format!("ih-sbx-canary-{}", std::process::id()));
    std::fs::write(&canary, "CANARY-SECRET").unwrap();

    let workdir = unique("wt");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("note.txt"), "alpha\n").unwrap();
    let socket_path = std::env::temp_dir().join(format!("ih-sbx.sock-{}", std::process::id()));
    let _ = std::fs::remove_file(&socket_path);

    let script = format!(
        "echo worktree=$(cat note.txt); \
         echo data > created.txt && echo WRITE_WORKTREE=ok; \
         cat /etc/hostname >/dev/null && echo SYSTEM_READ=ok; \
         echo HOME_READ=$(cat {canary} 2>&1 | head -c 80); \
         echo > {home}/.cache/ih-sbx-write-{pid}.txt 2>&1 && echo HOST_WRITE=ok || echo HOST_WRITE=refused; \
         rm {canary} 2>&1 && echo HOST_DELETE=ok || echo HOST_DELETE=refused; \
         cat {home}/.ssh/id_rsa 2>&1 | head -c 60; echo; \
         test -S {socket} 2>&1 && echo SOCKET=ok || echo SOCKET=refused; \
         printenv | wc -l",
        canary = canary.display(),
        home = home.display(),
        pid = std::process::id(),
        socket = socket_path.display(),
    );
    std::env::set_var("IH_HOST_ONLY_SECRET", "host-only-secret");
    let script = format!("{script}\nprintenv | tr '\\n' ' '; echo; printenv IH_HOST_ONLY_SECRET || echo HOST_ENV=absent");
    let (success, stdout, stderr) = run_in_sandbox(
        SecurityLevel::IsolatedNetwork,
        &workdir,
        &script,
        &[("IH_SECRET_CANARY".to_string(), "env-secret-42".to_string())],
    );
    std::env::remove_var("IH_HOST_ONLY_SECRET");
    println!("stdout={stdout}\nstderr={stderr}");
    assert!(success, "la sandbox doit s'exécuter : {stderr}");
    assert!(stdout.contains("worktree=alpha"), "lecture worktree");
    assert!(stdout.contains("WRITE_WORKTREE=ok"), "écriture worktree");
    assert!(
        stdout.contains("SYSTEM_READ=ok"),
        "lecture système autorisée"
    );
    assert!(
        !stdout.contains("CANARY-SECRET") || stdout.contains("HOME_READ="),
        "le canari de l'hôte ne doit pas être lisible"
    );
    assert!(
        !stdout.contains("CANARY-SECRET"),
        "fuite du fichier personnel : {stdout}"
    );
    assert!(stdout.contains("HOST_WRITE=refused"), "écriture extérieure");
    assert!(
        stdout.contains("HOST_DELETE=refused"),
        "suppression extérieure"
    );
    assert!(
        !stdout.contains("PRIVATE KEY"),
        "lecture de ~/.ssh ne doit pas être possible"
    );
    assert!(
        stdout.contains("SOCKET=refused"),
        "socket de l'hôte visible"
    );
    // Environnement reconstruit : le canari explicitement fourni est présent,
    // une variable de l'hôte n'est jamais héritée.
    assert!(
        stdout.contains("env-secret-42"),
        "la variable fournie explicitement doit être disponible"
    );
    assert!(
        !stdout.contains("host-only-secret"),
        "l'environnement de l'hôte ne doit pas être hérité"
    );
    // L'hôte n'a réellement rien vu : ni écriture, ni suppression.
    assert!(!home
        .join(".cache")
        .join(format!("ih-sbx-write-{}.txt", std::process::id()))
        .exists());
    assert!(canary.exists(), "le canari de l'hôte a été supprimé");

    let _ = std::fs::remove_file(&canary);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn reseau_desactive_au_niveau_isole_et_disponible_au_niveau_reseau() {
    if !ready() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let serve_dir = unique("serve");
    std::fs::create_dir_all(&serve_dir).unwrap();
    let mut server = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(&serve_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("serveur local");
    wait_port(port);

    let workdir = unique("net");
    std::fs::create_dir_all(&workdir).unwrap();
    let with_network = format!("curl -s -m 3 -o /dev/null -w 'HTTP=%{{http_code}}' http://127.0.0.1:{port}/ || echo NET=refused");
    let (_, stdout_l2, _) =
        run_in_sandbox(SecurityLevel::IsolatedNetwork, &workdir, &with_network, &[]);
    assert!(
        stdout_l2.contains("HTTP=200"),
        "réseau attendu : {stdout_l2}"
    );

    let (_, stdout_l1, _) = run_in_sandbox(SecurityLevel::Isolated, &workdir, &with_network, &[]);
    assert!(
        !stdout_l1.contains("HTTP=200"),
        "le niveau isolé doit couper le réseau : {stdout_l1}"
    );

    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&serve_dir);
    let _ = std::fs::remove_dir_all(&workdir);
}
