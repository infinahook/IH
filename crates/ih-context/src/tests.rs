//! Tests du crate sur de vrais dossiers temporaires.

use super::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ih-context-{prefix}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("création du dossier temporaire");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path.join(rel);
        std::fs::create_dir_all(path.parent().expect("dossier parent")).expect("dossier parent");
        std::fs::write(path, content).expect("écriture du fichier de test");
    }

    fn write_bytes(&self, rel: &str, content: &[u8]) {
        let path = self.path.join(rel);
        std::fs::create_dir_all(path.parent().expect("dossier parent")).expect("dossier parent");
        std::fs::write(path, content).expect("écriture du fichier de test");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn paths(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

fn find_skipped<'a>(context: &'a PreparedContext, path: &str) -> &'a SkippedFile {
    context
        .skipped
        .iter()
        .find(|skipped| skipped.path == path)
        .unwrap_or_else(|| panic!("{path} devrait être dans skipped : {:?}", context.skipped))
}

#[test]
fn options_par_defaut() {
    let options = ContextOptions::default();
    assert_eq!(options.max_files, DEFAULT_MAX_FILES);
    assert_eq!(options.max_bytes_per_file, DEFAULT_MAX_BYTES_PER_FILE);
    assert_eq!(options.max_total_bytes, DEFAULT_MAX_TOTAL_BYTES);
    assert!(!options.allow_sensitive);
    assert!(options.extra_exclusions.is_empty());
}

#[test]
fn exclusions_par_defaut_et_personnalisees() {
    let cases: [(&str, &str); 10] = [
        (".env", ".env"),
        (".env.local", ".env.*"),
        ("secrets/id_rsa", "id_rsa"),
        ("certs/a.pem", "*.pem"),
        ("x.key", "*.key"),
        ("node_modules/x.js", "node_modules/"),
        ("target/debug/x", "target/"),
        (".git/config", ".git/"),
        ("sub/.ssh/id_ed25519", "id_ed25519"),
        ("dist/app.js", "dist/"),
    ];
    for (path, expected_pattern) in cases {
        let reason =
            exclusion_reason(path, &[]).unwrap_or_else(|| panic!("{path} devrait être exclu"));
        assert!(
            reason.starts_with("exclu: "),
            "raison inattendue pour {path} : {reason}"
        );
        assert!(
            reason.contains(expected_pattern),
            "motif {expected_pattern} attendu pour {path}, obtenu {reason}"
        );
    }

    assert_eq!(exclusion_reason("src/main.rs", &[]), None);
    assert_eq!(exclusion_reason("src/node_modules_backup/x.js", &[]), None);
    assert_eq!(
        exclusion_reason("src/main.rs", &["*.rs".to_string()]),
        Some("exclu: *.rs".to_string())
    );
    assert_eq!(
        exclusion_reason("logs/app.log", &["*.log".to_string()]),
        Some("exclu: *.log".to_string())
    );
    assert_eq!(
        exclusion_reason("generated/x.rs", &["generated/".to_string()]),
        Some("exclu: generated/".to_string())
    );
    assert_eq!(
        exclusion_reason("src/main.rs", &["generated/".to_string()]),
        None
    );
    // Les dossiers sont comparés sans tenir compte de la casse.
    assert!(exclusion_reason("Target/debug/x", &[]).is_some());

    let defaults = default_exclusions();
    for pattern in [
        ".env",
        "*.pem",
        ".ssh/",
        "node_modules/",
        "target/",
        "__pycache__/",
    ] {
        assert!(
            defaults.iter().any(|item| item == pattern),
            "motif manquant : {pattern}"
        );
    }
}

#[test]
fn detection_de_contenu_sensible() {
    assert!(content_is_sensitive(
        "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n-----END PRIVATE KEY-----"
    )
    .is_some());
    assert!(content_is_sensitive("api_key = \"abc123def456\"").is_some());
    assert!(content_is_sensitive("api_key = \"...\"").is_some());
    assert!(content_is_sensitive("token: sk-...").is_some());
    assert!(content_is_sensitive("token: sk-abcdefghijklmnopqrstuvwxyz012345").is_some());
    assert!(content_is_sensitive("password = \"hunter2hunter2\"").is_some());
    assert!(content_is_sensitive("AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE").is_some());

    let ordinary = "fn main() {\n    println!(\"bonjour\");\n}\n\n\
                    Le mot password est décrit ici sans valeur affectée.\n";
    assert!(content_is_sensitive(ordinary).is_none());
    assert!(content_is_sensitive("").is_none());
}

#[test]
fn prepare_trois_fichiers_ordre_et_prompt() {
    let dir = TempDir::new("prepare");
    dir.write("src/main.rs", "fn main() {}\n");
    dir.write("Cargo.toml", "[package]\nname = \"demo\"\n");
    dir.write("README.md", "# Démonstration\n");

    let context = prepare(
        dir.path(),
        &paths(&["src/main.rs", "Cargo.toml", "README.md"]),
        &ContextOptions::default(),
    )
    .expect("contexte assemblé");

    assert_eq!(context.files.len(), 3);
    assert!(context.skipped.is_empty());
    let expected_total: usize = context.files.iter().map(|file| file.content.len()).sum();
    assert_eq!(context.total_bytes, expected_total);
    assert_eq!(
        context
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/main.rs", "Cargo.toml", "README.md"]
    );
    for file in &context.files {
        assert_eq!(file.bytes, file.content.len());
        assert!(!file.truncated);
        assert!(!file.sensitive);
        assert!(context.prompt_block.contains(&file.path));
        assert!(context.prompt_block.contains(&file.content));
    }
    assert!(context.prompt_block.starts_with(PROMPT_HEADER));
    let main_position = context.prompt_block.find("src/main.rs").unwrap();
    let cargo_position = context.prompt_block.find("Cargo.toml").unwrap();
    let readme_position = context.prompt_block.find("README.md").unwrap();
    assert!(main_position < cargo_position && cargo_position < readme_position);
    assert!(context.prompt_block.contains("octets)"));
}

#[test]
fn deduplication_des_chemins() {
    let dir = TempDir::new("dedup");
    dir.write("a.txt", "contenu\n");

    let context = prepare(
        dir.path(),
        &paths(&["a.txt", "./a.txt", "a.txt", "a.txt"]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].path, "a.txt");
    assert!(context.skipped.is_empty());
}

#[test]
fn fichier_exclu_jamais_dans_le_prompt() {
    let dir = TempDir::new("exclu");
    dir.write(".env", "TOP_SECRET=ne-doit-pas-sortir\n");
    dir.write("src/main.rs", "fn main() {}\n");

    let context = prepare(
        dir.path(),
        &paths(&[".env", "src/main.rs"]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].path, "src/main.rs");
    let skipped = find_skipped(&context, ".env");
    assert!(
        skipped.reason.starts_with("exclu: "),
        "raison : {}",
        skipped.reason
    );
    assert!(!context.prompt_block.contains(".env"));
    assert!(!context.prompt_block.contains("TOP_SECRET"));
}

#[test]
fn fichier_binaire_ecarte() {
    let dir = TempDir::new("binaire");
    dir.write_bytes("image.dat", b"\x89PNG\0donnees-binaires");
    dir.write("note.txt", "texte\n");

    let context = prepare(
        dir.path(),
        &paths(&["image.dat", "note.txt"]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].path, "note.txt");
    assert_eq!(find_skipped(&context, "image.dat").reason, BINARY_REASON);
    assert!(!context.prompt_block.contains("donnees-binaires"));
}

#[test]
fn troncature_utf8_propre() {
    let dir = TempDir::new("troncature");
    // 99 octets ASCII, puis un « é » de 2 octets à cheval sur la limite 100.
    let body = format!("{}é{}", "a".repeat(99), "b".repeat(50));
    dir.write("long.txt", &body);

    let options = ContextOptions {
        max_bytes_per_file: 100,
        ..ContextOptions::default()
    };
    let context = prepare(dir.path(), &paths(&["long.txt"]), &options).unwrap();
    let file = &context.files[0];
    assert!(file.truncated);
    assert!(file.bytes <= 100);
    assert_eq!(file.bytes, file.content.len());
    assert_eq!(context.total_bytes, file.bytes);
    assert!(!file.content.contains('\u{FFFD}'));
    assert!(std::str::from_utf8(file.content.as_bytes()).is_ok());
    // La coupe tombe avant le caractère multi-octets : 99 « a » exactement.
    assert_eq!(file.content, "a".repeat(99));
    assert!(context.prompt_block.contains("tronqué"));

    // À 101 octets, le « é » complet tient et la coupe reste valide.
    let options = ContextOptions {
        max_bytes_per_file: 101,
        ..ContextOptions::default()
    };
    let context = prepare(dir.path(), &paths(&["long.txt"]), &options).unwrap();
    let file = &context.files[0];
    assert_eq!(file.bytes, 101);
    assert!(file.content.ends_with('é'));
    assert!(!file.content.contains('\u{FFFD}'));
}

#[test]
fn budget_total_arret_explicite() {
    let dir = TempDir::new("budget");
    for index in 0..4 {
        dir.write(&format!("f{index}.txt"), &"x".repeat(40));
    }

    let options = ContextOptions {
        max_total_bytes: 100,
        ..ContextOptions::default()
    };
    let context = prepare(
        dir.path(),
        &paths(&["f0.txt", "f1.txt", "f2.txt", "f3.txt"]),
        &options,
    )
    .unwrap();

    assert_eq!(context.files.len(), 2);
    assert_eq!(context.total_bytes, 80);
    assert_eq!(find_skipped(&context, "f2.txt").reason, BUDGET_REASON);
    assert_eq!(find_skipped(&context, "f3.txt").reason, BUDGET_REASON);
    assert!(!context.prompt_block.contains("f2.txt"));
    assert!(!context.prompt_block.contains("f3.txt"));
}

#[test]
fn max_files_respecte() {
    let dir = TempDir::new("max-files");
    for index in 0..5 {
        dir.write(&format!("f{index}.txt"), "contenu\n");
    }

    let options = ContextOptions {
        max_files: 2,
        ..ContextOptions::default()
    };
    let context = prepare(
        dir.path(),
        &paths(&["f0.txt", "f1.txt", "f2.txt", "f3.txt", "f4.txt"]),
        &options,
    )
    .unwrap();

    assert_eq!(context.files.len(), 2);
    assert_eq!(context.files[0].path, "f0.txt");
    assert_eq!(context.files[1].path, "f1.txt");
    for path in ["f2.txt", "f3.txt", "f4.txt"] {
        assert_eq!(find_skipped(&context, path).reason, FILE_LIMIT_REASON);
    }
}

#[test]
fn sensible_exclu_puis_inclus_sur_confirmation() {
    let dir = TempDir::new("sensible");
    dir.write("config.txt", "api_key = \"abc123def456\"\n");

    let context = prepare(
        dir.path(),
        &paths(&["config.txt"]),
        &ContextOptions::default(),
    )
    .unwrap();
    assert!(context.files.is_empty());
    assert_eq!(
        find_skipped(&context, "config.txt").reason,
        SENSITIVE_REASON
    );
    assert!(context.prompt_block.is_empty());
    assert!(!context.prompt_block.contains("abc123def456"));

    let options = ContextOptions {
        allow_sensitive: true,
        ..ContextOptions::default()
    };
    let context = prepare(dir.path(), &paths(&["config.txt"]), &options).unwrap();
    assert_eq!(context.files.len(), 1);
    assert!(context.files[0].sensitive);
    assert!(context.files[0].content.contains("abc123def456"));
    assert!(context.skipped.is_empty());
}

#[test]
fn chemins_hors_workspace_refuses() {
    let dir = TempDir::new("evasion");
    dir.write("ok.txt", "ok\n");

    let context = prepare(
        dir.path(),
        &paths(&[
            "../etc/passwd",
            "/etc/passwd",
            "src/../secret.txt",
            "ok.txt",
        ]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].path, "ok.txt");
    for path in ["../etc/passwd", "/etc/passwd", "src/../secret.txt"] {
        assert_eq!(find_skipped(&context, path).reason, "hors workspace");
    }
    assert!(!context.prompt_block.contains("root:"));
    assert!(!context.prompt_block.contains("/etc/passwd"));
}

#[cfg(unix)]
#[test]
fn symlinks_sortants_refuses() {
    use std::os::unix::fs::symlink;

    let outside = TempDir::new("exterieur");
    outside.write("secret.txt", "TOP_SECRET_EXTERIEUR\n");
    let dir = TempDir::new("symlink");
    dir.write("ok.txt", "ok\n");
    symlink(
        outside.path().join("secret.txt"),
        dir.path().join("lien-fichier"),
    )
    .unwrap();
    symlink(outside.path(), dir.path().join("lien-dossier")).unwrap();

    let context = prepare(
        dir.path(),
        &paths(&["lien-fichier", "lien-dossier/secret.txt", "ok.txt"]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].path, "ok.txt");
    assert_eq!(
        find_skipped(&context, "lien-fichier").reason,
        "hors workspace"
    );
    assert_eq!(
        find_skipped(&context, "lien-dossier/secret.txt").reason,
        "hors workspace"
    );
    assert!(!context.prompt_block.contains("TOP_SECRET_EXTERIEUR"));
}

#[cfg(unix)]
#[test]
fn symlink_interne_ne_contourne_pas_les_exclusions() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("symlink-interne");
    dir.write(".env", "SECRET_INTERNE=1\n");
    dir.write("ok.txt", "contenu autorisé\n");
    symlink(dir.path().join(".env"), dir.path().join("lien-env")).unwrap();
    symlink(dir.path().join("ok.txt"), dir.path().join("lien-ok")).unwrap();

    let context = prepare(
        dir.path(),
        &paths(&["lien-env", "lien-ok"]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].path, "lien-ok");
    assert_eq!(context.files[0].content, "contenu autorisé\n");
    let skipped = find_skipped(&context, "lien-env");
    assert!(
        skipped.reason.starts_with("exclu: "),
        "raison : {}",
        skipped.reason
    );
    assert!(!context.prompt_block.contains("SECRET_INTERNE"));
}

#[test]
fn fichier_inexistant_signale_sans_erreur_globale() {
    let dir = TempDir::new("absent");
    dir.write("present.txt", "présent\n");

    let context = prepare(
        dir.path(),
        &paths(&["absent.txt", "present.txt"]),
        &ContextOptions::default(),
    )
    .unwrap();

    assert_eq!(context.files.len(), 1);
    assert!(find_skipped(&context, "absent.txt")
        .reason
        .starts_with("illisible: "));
    assert_eq!(context.files[0].path, "present.txt");
}

#[test]
fn match_mentions_score_et_exclusions() {
    let dir = TempDir::new("mentions");
    dir.write("src/main.rs", "fn main() {}\n");
    dir.write("docs/maintenance.md", "# maintenance\n");
    dir.write("Cargo.toml", "[package]\n");
    dir.write(".env", "SECRET=1\n");
    dir.write("target/debug/x", "artefact\n");

    let main = match_mentions(dir.path(), "main", 10, &[]).unwrap();
    let main_position = main
        .iter()
        .position(|candidate| candidate.path == "src/main.rs")
        .expect("src/main.rs proposé");
    let maintenance_position = main
        .iter()
        .position(|candidate| candidate.path == "docs/maintenance.md")
        .expect("docs/maintenance.md proposé");
    assert!(main_position < maintenance_position, "scores : {main:?}");
    assert!(main.iter().all(|candidate| !candidate.excluded));
    assert!(main
        .windows(2)
        .all(|window| window[0].score >= window[1].score));

    let cov = match_mentions(dir.path(), "cov", 10, &[]).unwrap();
    assert!(
        cov.iter().any(|candidate| candidate.path == "Cargo.toml"),
        "candidats : {cov:?}"
    );

    let limited = match_mentions(dir.path(), "main", 1, &[]).unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].path, "src/main.rs");

    let excluded = match_mentions(dir.path(), ".env", 10, &[]).unwrap();
    let env = excluded
        .iter()
        .find(|candidate| candidate.path == ".env")
        .expect("fichier exclu exact signalé");
    assert!(env.excluded);

    // `target/` est élagué par ProjectFs : la correspondance exacte doit tout
    // de même être signalée comme exclue, sans contenu.
    let pruned = match_mentions(dir.path(), "target/debug/x", 10, &[]).unwrap();
    let artifact = pruned
        .iter()
        .find(|candidate| candidate.path == "target/debug/x")
        .expect("chemin élagué exact signalé");
    assert!(artifact.excluded);

    // Une requête vide ou une limite nulle ne parcourt rien.
    assert!(match_mentions(dir.path(), "", 10, &[]).unwrap().is_empty());
    assert!(match_mentions(dir.path(), "main", 0, &[])
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
#[test]
fn match_mentions_dossiers_et_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("mentions-dossiers");
    dir.write("src/lib.rs", "pub fn x() {}\n");
    dir.write("src/nested/mod.rs", "\n");
    let outside = TempDir::new("mentions-exterieur");
    outside.write("secret.txt", "\n");
    symlink(outside.path(), dir.path().join("evade")).unwrap();

    let candidates = match_mentions(dir.path(), "src", 10, &[]).unwrap();
    assert!(candidates
        .iter()
        .any(|candidate| candidate.path == "src" && candidate.is_dir));

    // Le parcours ne suit pas les symlinks sortants.
    let hidden = match_mentions(dir.path(), "secret", 10, &[]).unwrap();
    assert!(hidden
        .iter()
        .all(|candidate| candidate.path != "evade/secret.txt"));
}

#[test]
fn list_files_exclusions_et_limite() {
    let dir = TempDir::new("liste");
    dir.write("src/a.rs", "fn a() {}\n");
    dir.write("README.md", "# readme\n");
    dir.write("target/debug/x", "binaire\n");
    dir.write(".git/config", "git\n");
    dir.write("node_modules/pkg/index.js", "js\n");
    dir.write("dist/app.js", "js\n");
    dir.write("build/out.bin", "\n");
    dir.write(".env", "SECRET=1\n");

    let files = list_files(dir.path(), 100, &[]).unwrap();
    assert!(files.iter().any(|path| path == "src/a.rs"));
    assert!(files.iter().any(|path| path == "README.md"));
    for path in [
        "target/debug/x",
        ".git/config",
        "node_modules/pkg/index.js",
        "dist/app.js",
        "build/out.bin",
        ".env",
    ] {
        assert!(
            files.iter().all(|file| file != path),
            "{path} ne devrait pas être listé"
        );
    }

    let limited = list_files(dir.path(), 1, &[]).unwrap();
    assert_eq!(limited.len(), 1);
    assert!(list_files(dir.path(), 0, &[]).unwrap().is_empty());

    let extra = list_files(dir.path(), 100, &["*.md".to_string()]).unwrap();
    assert!(extra.iter().all(|path| path != "README.md"));
    assert!(extra.iter().any(|path| path == "src/a.rs"));
}

#[test]
fn contexte_vide_et_serialisation() {
    let dir = TempDir::new("vide");
    let context = prepare(dir.path(), &[], &ContextOptions::default()).unwrap();
    assert!(context.files.is_empty());
    assert!(context.prompt_block.is_empty());
    assert_eq!(context.total_bytes, 0);

    dir.write("a.txt", "bonjour\n");
    let context = prepare(dir.path(), &paths(&["a.txt"]), &ContextOptions::default()).unwrap();
    let json = serde_json::to_string(&context).unwrap();
    let back: PreparedContext = serde_json::from_str(&json).unwrap();
    assert_eq!(back.files.len(), 1);
    assert_eq!(back.files[0].path, "a.txt");
    assert_eq!(back.total_bytes, context.total_bytes);
    assert_eq!(back.prompt_block, context.prompt_block);
}
