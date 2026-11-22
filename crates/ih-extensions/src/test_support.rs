//! Aides de test partagées : dossiers temporaires réels, nettoyés à la fin.

use std::fs;
use std::path::{Path, PathBuf};

/// Dossier temporaire unique, supprimé récursivement au `Drop`.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "ih-extensions-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("création du dossier temporaire");
        // Canonicalise pour que /tmp -> /private/tmp (macOS) ne fausse pas les
        // comparaisons de chemins dans les tests.
        let path = fs::canonicalize(&path).expect("canonicalisation du dossier temporaire");
        Self { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Écrit un fichier en créant ses parents.
pub(crate) fn write_file(path: &Path, contents: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("création des parents du fichier de test");
    }
    fs::write(path, contents).expect("écriture du fichier de test");
}
