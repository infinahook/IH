//! Vérifications réelles de l'isolation système (bubblewrap).
//!
//! Ces tests exigent un moteur **fonctionnel** : ils sont marqués `#[ignore]`
//! pour que `cargo test --workspace` ne les présente jamais comme réussis sans
//! les avoir exécutés. Exécution explicite :
//!
//! ```text
//! cargo test -p ih-agents --test sandbox_runtime -- --ignored --test-threads=1
//! ```
//!
//! Les canaris sont créés dans un dossier temporaire, **jamais** dans le vrai
//! `$HOME` : le test ne touche ni `~/.cache`, ni `~/.ssh`, ni aucun fichier
//! personnel.
//!
//! Quand `IH_REQUIRE_SANDBOX=1` est défini (job CI dédié), l'absence de sandbox
//! est une **erreur**, jamais un succès silencieux.

use ih_agents::sandbox::{self, SecurityLevel};
use ih_core::config::AgentKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn ready() -> bool {
    sandbox::availability().functional
}

/// Sandbox indispensable dans ce contexte (CI dédiée) : échec explicite.
fn require_sandbox() {
    if std::env::var("IH_REQUIRE_SANDBOX").ok().as_deref() != Some("1") {
        return;
    }
    if !ready() {
        panic!(
            "IH_REQUIRE_SANDBOX=1 mais la sandbox n'est pas fonctionnelle : {:?}",
            sandbox::availability()
        );
    }
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
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn la_sandbox_bloque_les_acces_hors_worktree() {
    require_sandbox();
    if !ready() {
        eprintln!("IGNORÉ: bubblewrap non fonctionnel sur cette machine");
        return;
    }

    // Canaris entièrement temporaires : aucun fichier personnel n'est touché.
    let canary_root = unique("canaris");
    let workdir = unique("wt");
    std::fs::create_dir_all(canary_root.join(".ssh")).unwrap();
    std::fs::create_dir_all(canary_root.join(".aws")).unwrap();
    std::fs::create_dir_all(&workdir).unwrap();
    let ssh_key = canary_root.join(".ssh/id_rsa");
    let aws_creds = canary_root.join(".aws/credentials");
    let dotfile = canary_root.join(".secret.txt");
    std::fs::write(&ssh_key, "CANARY-SSH-KEY").unwrap();
    std::fs::write(&aws_creds, "CANARY-AWS").unwrap();
    std::fs::write(&dotfile, "CANARY-DOTFILE").unwrap();
    std::fs::write(workdir.join("note.txt"), "alpha\n").unwrap();

    let script = format!(
        "echo worktree=$(cat note.txt); \
         echo data > created.txt && echo WRITE_WORKTREE=ok; \
         cat /etc/hostname >/dev/null && echo SYSTEM_READ=ok; \
         echo SSH_READ=$(cat {ssh} 2>&1 | head -c 40); \
         echo AWS_READ=$(cat {aws} 2>&1 | head -c 40); \
         echo DOT_READ=$(cat {dot} 2>&1 | head -c 40); \
         echo home=$(printenv HOME); \
         echo > {canary}/hors-worktree.txt 2>&1 && echo HOST_WRITE=ok || echo HOST_WRITE=refused; \
         rm {ssh} 2>&1 && echo HOST_DELETE=ok || echo HOST_DELETE=refused",
        ssh = ssh_key.display(),
        aws = aws_creds.display(),
        dot = dotfile.display(),
        canary = canary_root.display(),
    );
    let (success, stdout, stderr) =
        run_in_sandbox(SecurityLevel::IsolatedNetwork, &workdir, &script, &[]);
    println!("stdout={stdout}\nstderr={stderr}");

    assert!(success, "la sandbox doit s'exécuter : {stderr}");
    assert!(stdout.contains("worktree=alpha"), "lecture du worktree");
    assert!(stdout.contains("WRITE_WORKTREE=ok"), "écriture du worktree");
    assert!(
        stdout.contains("SYSTEM_READ=ok"),
        "lecture système autorisée"
    );
    assert!(
        stdout.contains("home=/home/agent"),
        "home privé de la sandbox"
    );
    // Aucun canari personnel ne doit apparaître dans la sortie.
    for secret in ["CANARY-SSH-KEY", "CANARY-AWS", "CANARY-DOTFILE"] {
        assert!(
            !stdout.contains(secret),
            "fuite du canari {secret} : {stdout}"
        );
    }
    assert!(stdout.contains("HOST_WRITE=refused"), "écriture extérieure");
    assert!(
        stdout.contains("HOST_DELETE=refused"),
        "suppression extérieure"
    );
    // L'hôte n'a réellement rien vu.
    assert!(!canary_root.join("hors-worktree.txt").exists());
    assert!(ssh_key.exists(), "le canari de l'hôte a été supprimé");

    let _ = std::fs::remove_dir_all(&canary_root);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn lecriture_hors_worktree_est_refusee_meme_en_tmp() {
    require_sandbox();
    if !ready() {
        eprintln!("IGNORÉ: bubblewrap non fonctionnel sur cette machine");
        return;
    }
    let workdir = unique("tmpwt");
    std::fs::create_dir_all(&workdir).unwrap();
    let host_tmp = unique("tmpcanari");
    std::fs::create_dir_all(&host_tmp).unwrap();
    let canary = host_tmp.join("canari.txt");
    std::fs::write(&canary, "intact\n").unwrap();

    let script = format!(
        "cat {canary} 2>&1 | head -c 40; echo; \
         echo pirate > {canary} 2>&1 && echo TMP_WRITE=ok || echo TMP_WRITE=refused; ls /tmp | head -3",
        canary = canary.display()
    );
    let (success, stdout, stderr) = run_in_sandbox(SecurityLevel::Isolated, &workdir, &script, &[]);
    assert!(success, "sandbox: {stderr}");
    assert!(!stdout.contains("intact"), "le /tmp de l'hôte est visible");
    assert!(stdout.contains("TMP_WRITE=refused"), "écriture /tmp hôte");
    assert_eq!(std::fs::read_to_string(&canary).unwrap(), "intact\n");

    let _ = std::fs::remove_dir_all(&host_tmp);
    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn reseau_desactive_au_niveau_isole_et_disponible_au_niveau_reseau() {
    require_sandbox();
    if !ready() {
        eprintln!("IGNORÉ: bubblewrap non fonctionnel sur cette machine");
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
    let with_network = format!(
        "curl -s -m 3 -o /dev/null -w 'HTTP=%{{http_code}}' http://127.0.0.1:{port}/ || echo NET=refused"
    );
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
