//! Politique de ressources : cible matérielle 16 Go de RAM / GTX 1660 Ti.
//!
//! Ces limites sont appliquées par le noyau, pas seulement affichées dans
//! l'interface : tout dépassement est refusé avec `IhError::Limit`.

use crate::error::{IhError, IhResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourcePolicy {
    /// Nombre maximal de sessions d'agent actives simultanément.
    pub max_concurrent_agents: u8,
    /// Taille maximale de sortie conservée par session (octets).
    pub max_output_bytes_per_session: u64,
    /// Nombre d'éléments par page d'historique.
    pub transcript_page_size: u32,
    /// Lignes conservées par terminal (scrollback).
    pub terminal_scrollback_lines: u32,
    /// Ne jamais charger un modèle local au démarrage.
    pub preload_local_models: bool,
    /// Mémoire maximale allouée au cache d'embeddings (Mo).
    pub embedding_cache_mb: u32,
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self {
            max_concurrent_agents: 2,
            max_output_bytes_per_session: 4 * 1024 * 1024,
            transcript_page_size: 50,
            terminal_scrollback_lines: 5_000,
            preload_local_models: false,
            embedding_cache_mb: 256,
        }
    }
}

impl ResourcePolicy {
    /// Plafond absolu, non contournable par la configuration utilisateur.
    pub const HARD_MAX_AGENTS: u8 = 4;
    pub const MAX_OUTPUT_HARD_CAP: u64 = 64 * 1024 * 1024;

    /// Valide la politique et renvoie une erreur explicite.
    pub fn validate(&self) -> IhResult<()> {
        if self.max_concurrent_agents == 0 {
            return Err(IhError::Config(
                "max_concurrent_agents doit être ≥ 1".into(),
            ));
        }
        if self.max_concurrent_agents > Self::HARD_MAX_AGENTS {
            return Err(IhError::Limit(format!(
                "max_concurrent_agents dépasse le plafond absolu ({})",
                Self::HARD_MAX_AGENTS
            )));
        }
        if self.max_output_bytes_per_session == 0
            || self.max_output_bytes_per_session > Self::MAX_OUTPUT_HARD_CAP
        {
            return Err(IhError::Config(format!(
                "max_output_bytes_per_session doit être entre 1 et {}",
                Self::MAX_OUTPUT_HARD_CAP
            )));
        }
        if self.transcript_page_size == 0 || self.transcript_page_size > 500 {
            return Err(IhError::Config(
                "transcript_page_size doit être entre 1 et 500".into(),
            ));
        }
        if self.terminal_scrollback_lines > 100_000 {
            return Err(IhError::Config(
                "terminal_scrollback_lines dépasse 100000".into(),
            ));
        }
        if self.embedding_cache_mb > 2048 {
            return Err(IhError::Config("embedding_cache_mb dépasse 2048".into()));
        }
        Ok(())
    }

    /// Ramène les valeurs hors bornes dans les limites sûres.
    pub fn clamped(mut self) -> Self {
        self.max_concurrent_agents = self.max_concurrent_agents.clamp(1, Self::HARD_MAX_AGENTS);
        self.max_output_bytes_per_session = self
            .max_output_bytes_per_session
            .clamp(64 * 1024, Self::MAX_OUTPUT_HARD_CAP);
        self.transcript_page_size = self.transcript_page_size.clamp(1, 500);
        self.terminal_scrollback_lines = self.terminal_scrollback_lines.min(100_000);
        self.embedding_cache_mb = self.embedding_cache_mb.min(2048);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valeurs_par_defaut_conformes_a_la_cible_materielle() {
        let p = ResourcePolicy::default();
        assert_eq!(p.max_concurrent_agents, 2);
        assert!(!p.preload_local_models);
        p.validate().unwrap();
    }

    #[test]
    fn refus_au_dela_du_plafond_absolu() {
        let p = ResourcePolicy {
            max_concurrent_agents: 8,
            ..Default::default()
        };
        assert!(p.validate().is_err());
        assert_eq!(
            p.clamped().max_concurrent_agents,
            ResourcePolicy::HARD_MAX_AGENTS
        );
    }

    #[test]
    fn zero_session_refuse() {
        let p = ResourcePolicy {
            max_concurrent_agents: 0,
            ..Default::default()
        };
        assert!(p.validate().is_err());
        assert_eq!(p.clamped().max_concurrent_agents, 1);
    }
}
