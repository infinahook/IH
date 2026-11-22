//! Emplacements disque d'IH.
//!
//! Tout est dérivé d'une racine unique, remplaçable par `IH_HOME` pour les
//! tests d'intégration. Les répertoires sont créés en 0700 : la base contient
//! l'historique des conversations et ne doit pas être lisible par les autres
//! comptes de la machine.

use crate::error::{IhError, IhResult};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// Résout la racine applicative : `IH_HOME`, sinon `$XDG_CONFIG_HOME/ih`.
    pub fn resolve() -> IhResult<Self> {
        if let Some(custom) = std::env::var_os("IH_HOME") {
            if !custom.is_empty() {
                return Ok(Self::with_root(PathBuf::from(custom)));
            }
        }
        let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(xdg)
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".config")
        } else {
            return Err(IhError::Config(
                "impossible de localiser le répertoire de configuration (HOME absent)".into(),
            ));
        };
        Ok(Self::with_root(base.join("ih")))
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir().join("ih.sqlite3")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub fn plugins_dir(&self) -> PathBuf {
        self.root.join("plugins")
    }

    /// Crée l'arborescence avec des permissions restrictives.
    pub fn ensure(&self) -> IhResult<()> {
        for dir in [
            self.root.clone(),
            self.data_dir(),
            self.log_dir(),
            self.cache_dir(),
            self.backups_dir(),
            self.staging_dir(),
            self.skills_dir(),
            self.plugins_dir(),
        ] {
            std::fs::create_dir_all(&dir)?;
            restrict_permissions(&dir)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> IhResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> IhResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_sont_coherents() {
        let p = AppPaths::with_root("/tmp/ih-test");
        assert_eq!(p.config_file(), PathBuf::from("/tmp/ih-test/config.toml"));
        assert_eq!(p.db_path(), PathBuf::from("/tmp/ih-test/data/ih.sqlite3"));
        assert!(p.skills_dir().starts_with("/tmp/ih-test"));
    }

    #[test]
    fn ensure_cree_les_repertoires_et_restreint_les_droits() {
        let tmp = std::env::temp_dir().join(format!("ih-paths-{}", uuid::Uuid::new_v4()));
        let p = AppPaths::with_root(&tmp);
        p.ensure().unwrap();
        assert!(p.db_path().parent().unwrap().exists());
        assert!(p.log_dir().exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(p.data_dir())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o700,
                "le répertoire de données doit être privé"
            );
        }
        std::fs::remove_dir_all(&tmp).ok();
    }
}
