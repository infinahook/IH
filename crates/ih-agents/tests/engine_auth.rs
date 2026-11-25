//! Authentification du moteur de sandbox : un faux `bwrap` ne doit jamais être
//! accepté, même lorsqu'il est désigné explicitement pour les tests.
//!
//! `IH_SANDBOX_ENGINE` n'est honorée qu'en build de test/débogage **et** le
//! moteur doit passer une sonde négative (écriture hors sandbox refusée) ainsi
//! qu'un contrôle de propriété (binaire root non modifiable en production).
//!
//! Ce fichier n'exécute aucun agent : il ne teste que la validation du moteur.

use ih_agents::sandbox::{self, SecurityLevel};
use ih_core::config::AgentKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard;

impl EnvGuard {
    fn set(value: &Path) -> Self {
        std::env::set_var("IH_SANDBOX_ENGINE", value);
        Self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var("IH_SANDBOX_ENGINE");
    }
}

fn unique(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ih-engine-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn make_engine(tag: &str, body: &str, mode: u32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = unique(tag);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

/// Un faux moteur qui ignore les drapeaux de sécurité est refusé : la sonde
/// négative constate que l'écriture hors sandbox réussit.
#[test]
fn un_faux_bwrap_qui_ignore_les_drapeaux_est_refuse() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fake = make_engine(
        "faux",
        "#!/bin/sh\nwhile [ \"$1\" != \"--\" ] && [ $# -gt 0 ]; do shift; done\nshift\nexec \"$@\"\n",
        0o755,
    );
    let _guard = EnvGuard::set(&fake);

    let report = sandbox::availability();
    println!("rapport : {report:?}");
    assert!(
        !report.functional,
        "un faux moteur ne doit jamais être considéré fonctionnel"
    );
    let error = report.error.unwrap_or_default();
    assert!(
        error.contains("n'isole pas") || error.contains("hors de la sandbox"),
        "diagnostic inattendu : {error}"
    );

    // Et la construction de commande refuse tout lancement « sandboxé ».
    let workdir = std::env::temp_dir().join(format!("ih-engine-wt-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).unwrap();
    let command = sandbox::sandboxed_command(
        SecurityLevel::Isolated,
        &AgentKind::Custom,
        Path::new("/bin/sh"),
        &[],
        &workdir,
        &[],
        "engine-test",
    );
    assert!(command.is_err(), "aucune commande sandboxée ne doit sortir");
    let _ = std::fs::remove_dir_all(&workdir);
    let _ = std::fs::remove_file(&fake);
}

/// Un moteur inscriptible par le groupe ou le monde est refusé même s'il
/// répond correctement aux sondes (contrôle de propriété).
#[test]
fn un_moteur_modifiable_par_tous_est_refuse() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let fake = make_engine("partage", "#!/bin/sh\nexec \"$@\"\n", 0o777);
    let _guard = EnvGuard::set(&fake);

    let report = sandbox::availability();
    assert!(!report.functional);
    let error = report.error.unwrap_or_default();
    assert!(
        error.contains("inscriptible par le groupe ou le monde"),
        "diagnostic inattendu : {error}"
    );
    let _ = std::fs::remove_file(&fake);
}

/// Un chemin relatif est refusé : le moteur doit être un binaire absolu.
#[test]
fn un_moteur_relatif_est_refuse() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _guard = EnvGuard::set(Path::new("bwrap-relatif"));
    let report = sandbox::availability();
    assert!(!report.functional);
    let error = report.error.unwrap_or_default();
    assert!(error.contains("relatif"), "diagnostic inattendu : {error}");
}

/// La surcharge n'est honorée que dans les builds de test/débogage : le code
/// de production la neutralise (`cfg!(debug_assertions)`).
#[test]
fn la_surcharge_n_est_honoree_qu_en_debug() {
    if !cfg!(debug_assertions) {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let fake = make_engine("release", "#!/bin/sh\nexit 0\n", 0o755);
        let _guard = EnvGuard::set(&fake);
        let report = sandbox::availability();
        assert_ne!(
            report
                .path
                .as_deref()
                .map(|path| path.contains("ih-engine-release")),
            Some(true),
            "la surcharge doit être ignorée hors debug"
        );
        let _ = std::fs::remove_file(&fake);
    } else {
        // En debug (dont `cargo test`), la surcharge est active : c'est elle qui
        // permet de simuler une machine sans moteur dans les tests suivants.
        let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let absent = unique("absent");
        let _guard = EnvGuard::set(&absent);
        let report = sandbox::availability();
        assert!(!report.functional);
        let error = report.error.unwrap_or_default();
        assert!(
            error.contains("introuvable") || error.contains("inaccessible"),
            "diagnostic inattendu : {error}"
        );
    }
}
