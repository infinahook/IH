//! Protection du dossier personnel : un exécutable utilisateur ne doit jamais
//! faire entrer `$HOME` (ni `~/.ssh`, `~/.aws`, `~/.config`) dans la sandbox.
//!
//! Le test utilise un **faux `$HOME`** temporaire : aucun fichier personnel réel
//! n'est lu, créé ou modifié. `IH_REQUIRE_SANDBOX=1` transforme l'absence de
//! moteur en échec (job CI dédié) au lieu d'un succès silencieux.

use ih_agents::sandbox::{self, SecurityLevel};
use ih_core::config::AgentKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// `HOME` est un état global du processus : les tests de ce fichier se
/// sérialisent.
static HOME_LOCK: Mutex<()> = Mutex::new(());

struct HomeGuard {
    previous: Option<std::ffi::OsString>,
}

impl HomeGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", path);
        Self { previous }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
}

fn ready() -> bool {
    sandbox::availability().functional
}

fn require_sandbox() {
    if std::env::var("IH_REQUIRE_SANDBOX").ok().as_deref() == Some("1") && !ready() {
        panic!(
            "IH_REQUIRE_SANDBOX=1 mais la sandbox n'est pas fonctionnelle : {:?}",
            sandbox::availability()
        );
    }
}

fn unique(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ih-home-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// Crée un faux dossier personnel avec des canaris synthétiques.
fn fake_home(root: &Path) {
    std::fs::create_dir_all(root.join(".ssh")).unwrap();
    std::fs::create_dir_all(root.join(".aws")).unwrap();
    std::fs::create_dir_all(root.join(".config")).unwrap();
    std::fs::create_dir_all(root.join(".local/share")).unwrap();
    std::fs::create_dir_all(root.join(".local/bin")).unwrap();
    std::fs::write(root.join(".ssh/id_rsa"), "CANARY-SSH-PRIVATE").unwrap();
    std::fs::write(root.join(".aws/credentials"), "CANARY-AWS-CREDS").unwrap();
    std::fs::write(root.join(".config/secret.toml"), "CANARY-CONFIG").unwrap();
    std::fs::write(root.join(".local/share/secret"), "CANARY-LOCALSHARE").unwrap();
}

fn run_plan(
    level: SecurityLevel,
    executable: &Path,
    workdir: &Path,
    script_args: &[String],
) -> (bool, String, String) {
    let plan = sandbox::sandboxed_command(
        level,
        &AgentKind::Custom,
        executable,
        script_args,
        workdir,
        &[],
        "home-test",
        None,
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

/// Un exécutable placé **directement** dans le dossier personnel est refusé :
/// le monter exposerait tout le home (lecture seule comprise).
#[test]
fn un_executable_a_la_racine_du_home_est_refuse() {
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = unique("refus");
    let home = root.join("home");
    fake_home(&home);
    let executable = home.join("mon-agent");
    std::fs::write(&executable, "#!/bin/sh\necho agent\n").unwrap();
    let workdir = root.join("wt");
    std::fs::create_dir_all(&workdir).unwrap();
    let _guard = HomeGuard::set(&home);

    let error = sandbox::sandboxed_command(
        SecurityLevel::Isolated,
        &AgentKind::Custom,
        &executable,
        &[],
        &workdir,
        &[],
        "home-refus",
        None,
    )
    .err()
    .map(|error| error.to_string())
    .expect("un exécutable à la racine du home doit être refusé");
    println!("refus obtenu : {error}");
    assert!(
        error.contains("isolation impossible"),
        "message de refus inattendu : {error}"
    );
    assert!(
        error.contains("dossier personnel"),
        "le refus doit nommer le dossier personnel : {error}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Un exécutable dans un sous-dossier dédié reste utilisable, et **aucun**
/// canari du dossier personnel n'est visible depuis la sandbox.
#[test]
#[ignore = "bubblewrap fonctionnel requis : exécuter avec --ignored"]
fn un_executable_dans_un_sous_dossier_ne_divulgue_aucun_canari() {
    require_sandbox();
    assert!(
        ready(),
        "ce test exige une sandbox fonctionnelle : {:?}",
        sandbox::availability()
    );
    let _lock = HOME_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = unique("sous-dossier");
    let home = root.join("home");
    fake_home(&home);
    let executable = home.join(".local/bin/mon-agent");
    let script = format!(
        "echo HOME=$(printenv HOME); \
         for f in {ssh} {aws} {config} {local}; do \
           if cat \"$f\" >/dev/null 2>&1; then echo LEAK=$(basename \"$f\"); else echo ABSENT=$(basename \"$f\"); fi; \
         done; \
         ls {home} 2>&1 | head -3",
        ssh = home.join(".ssh/id_rsa").display(),
        aws = home.join(".aws/credentials").display(),
        config = home.join(".config/secret.toml").display(),
        local = home.join(".local/share/secret").display(),
        home = home.display()
    );
    std::fs::write(&executable, format!("#!/bin/sh\n{script}")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let workdir = root.join("wt");
    std::fs::create_dir_all(&workdir).unwrap();
    let _guard = HomeGuard::set(&home);

    let (success, stdout, stderr) = run_plan(SecurityLevel::Isolated, &executable, &workdir, &[]);
    println!("stdout={stdout}\nstderr={stderr}");
    assert!(success, "la sandbox doit s'exécuter : {stderr}");
    assert!(stdout.contains("HOME=/home/agent"), "home privé : {stdout}");
    for canary in [
        "CANARY-SSH-PRIVATE",
        "CANARY-AWS-CREDS",
        "CANARY-CONFIG",
        "CANARY-LOCALSHARE",
    ] {
        assert!(
            !stdout.contains(canary),
            "fuite du canari {canary} : {stdout}"
        );
    }
    // Les chemins personnels ne sont pas montés du tout.
    assert!(
        stdout.contains("ABSENT=id_rsa"),
        "secret ssh visible : {stdout}"
    );
    assert!(
        stdout.contains("ABSENT=credentials"),
        "secret aws visible : {stdout}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
