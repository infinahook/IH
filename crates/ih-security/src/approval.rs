//! Registre d'approbations : chaque action sensible crée une demande, puis
//! un « ticket » à usage unique est exigé par l'exécuteur.
//!
//! Modèle : `Pending → Approved → Consumed`, avec expiration automatique.
//! Une approbation ne peut jamais être réutilisée pour une seconde action.

use crate::risk::RiskLevel;
use chrono::{DateTime, Duration, Utc};
use ih_core::{ApprovalId, IhError, IhResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Consumed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    /// `shell`, `fs_write`, `network`, `mcp_tool`, `install`, `secrets`
    pub category: String,
    pub summary: String,
    pub detail: String,
    pub risk: RiskLevel,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub state: ApprovalState,
}

impl ApprovalRequest {
    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

/// Preuve qu'une action a été explicitement approuvée pour cet usage unique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalTicket {
    pub id: ApprovalId,
    pub category: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

impl ApprovalDecision {
    pub fn parse(value: &str) -> IhResult<Self> {
        match value {
            "approve" | "approved" => Ok(Self::Approve),
            "deny" | "denied" => Ok(Self::Deny),
            other => Err(IhError::Invalid(format!(
                "décision d'approbation inconnue : {other}"
            ))),
        }
    }
}

pub struct ApprovalRegistry {
    pending: Mutex<HashMap<String, ApprovalRequest>>,
    ttl: Duration,
}

impl ApprovalRegistry {
    /// TTL par défaut : 5 minutes.
    pub fn new(ttl: Duration) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    pub fn default_ttl() -> Self {
        Self::new(Duration::minutes(5))
    }

    /// Crée une demande d'approbation et renvoie l'identifiant à suivre.
    pub fn ask(
        &self,
        category: &str,
        summary: &str,
        detail: &str,
        risk: RiskLevel,
    ) -> IhResult<ApprovalRequest> {
        if category.trim().is_empty() || summary.trim().is_empty() {
            return Err(IhError::Invalid(
                "une demande d'approbation exige une catégorie et un résumé".into(),
            ));
        }
        let now = Utc::now();
        let request = ApprovalRequest {
            id: ApprovalId::new(),
            category: category.to_string(),
            summary: summary.to_string(),
            detail: detail.to_string(),
            risk,
            created_at: now,
            expires_at: now + self.ttl,
            state: ApprovalState::Pending,
        };
        let mut guard = self.lock()?;
        guard.insert(request.id.to_string(), request.clone());
        Ok(request)
    }

    /// Décision de l'utilisateur (approuver ou refuser).
    pub fn resolve(&self, id: &str, decision: ApprovalDecision) -> IhResult<ApprovalRequest> {
        let mut guard = self.lock()?;
        let request = guard
            .get_mut(id)
            .ok_or_else(|| IhError::NotFound(format!("approbation {id}")))?;
        let now = Utc::now();
        if request.state != ApprovalState::Pending {
            return Err(IhError::Conflict(format!(
                "approbation {id} déjà traitée ({:?})",
                request.state
            )));
        }
        if request.is_expired(now) {
            request.state = ApprovalState::Expired;
            return Err(IhError::Conflict(format!("approbation {id} expirée")));
        }
        request.state = match decision {
            ApprovalDecision::Approve => ApprovalState::Approved,
            ApprovalDecision::Deny => ApprovalState::Denied,
        };
        Ok(request.clone())
    }

    /// Consomme une approbation approuvée et renvoie le ticket d'exécution.
    pub fn consume(&self, id: &str, category: &str) -> IhResult<ApprovalTicket> {
        let mut guard = self.lock()?;
        let request = guard
            .get_mut(id)
            .ok_or_else(|| IhError::NotFound(format!("approbation {id}")))?;
        let now = Utc::now();
        if request.is_expired(now) {
            request.state = ApprovalState::Expired;
            return Err(IhError::Denied(format!("approbation {id} expirée")));
        }
        match request.state {
            ApprovalState::Approved => {
                if request.category != category {
                    return Err(IhError::Denied(format!(
                        "l'approbation {id} concerne « {} » et non « {category} »",
                        request.category
                    )));
                }
                request.state = ApprovalState::Consumed;
                Ok(ApprovalTicket {
                    id: request.id.clone(),
                    category: request.category.clone(),
                })
            }
            ApprovalState::Consumed => {
                Err(IhError::Denied(format!("approbation {id} déjà consommée")))
            }
            other => Err(IhError::Denied(format!(
                "approbation {id} non utilisable ({other:?})"
            ))),
        }
    }

    pub fn list_pending(&self) -> IhResult<Vec<ApprovalRequest>> {
        let guard = self.lock()?;
        let mut list: Vec<_> = guard
            .values()
            .filter(|r| r.state == ApprovalState::Pending)
            .cloned()
            .collect();
        list.sort_by_key(|r| r.created_at);
        Ok(list)
    }

    pub fn get(&self, id: &str) -> IhResult<Option<ApprovalRequest>> {
        Ok(self.lock()?.get(id).cloned())
    }

    /// Marque comme expirées les demandes dépassées et purge l'historique ancien.
    pub fn sweep(&self) -> IhResult<usize> {
        let mut guard = self.lock()?;
        let now = Utc::now();
        let mut expired = 0;
        for request in guard.values_mut() {
            if request.state == ApprovalState::Pending && request.is_expired(now) {
                request.state = ApprovalState::Expired;
                expired += 1;
            }
        }
        guard.retain(|_, r| {
            r.state == ApprovalState::Pending || now - r.created_at < Duration::hours(24)
        });
        Ok(expired)
    }

    fn lock(&self) -> IhResult<std::sync::MutexGuard<'_, HashMap<String, ApprovalRequest>>> {
        self.pending
            .lock()
            .map_err(|_| IhError::Internal("registre d'approbations empoisonné".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_approuver_puis_consommer_une_seule_fois() {
        let reg = ApprovalRegistry::default_ttl();
        let req = reg
            .ask(
                "shell",
                "cargo build",
                "cargo build --release",
                RiskLevel::Low,
            )
            .unwrap();
        assert_eq!(req.state, ApprovalState::Pending);
        assert_eq!(reg.list_pending().unwrap().len(), 1);

        reg.resolve(req.id.as_str(), ApprovalDecision::Approve)
            .unwrap();
        let ticket = reg.consume(req.id.as_str(), "shell").unwrap();
        assert_eq!(ticket.id, req.id);

        // deuxième consommation refusée
        assert!(reg.consume(req.id.as_str(), "shell").is_err());
        assert!(reg.list_pending().unwrap().is_empty());
    }

    #[test]
    fn refus_interdit_la_consommation() {
        let reg = ApprovalRegistry::default_ttl();
        let req = reg
            .ask("install", "paquet deb", "apt install", RiskLevel::High)
            .unwrap();
        reg.resolve(req.id.as_str(), ApprovalDecision::Deny)
            .unwrap();
        assert!(reg.consume(req.id.as_str(), "install").is_err());
    }

    #[test]
    fn categorie_differente_refusee() {
        let reg = ApprovalRegistry::default_ttl();
        let req = reg.ask("shell", "ls", "ls", RiskLevel::Low).unwrap();
        reg.resolve(req.id.as_str(), ApprovalDecision::Approve)
            .unwrap();
        assert!(reg.consume(req.id.as_str(), "network").is_err());
        // l'approbation reste utilisable pour la bonne catégorie
        assert!(reg.consume(req.id.as_str(), "shell").is_ok());
    }

    #[test]
    fn expiration_bloque_la_consommation() {
        let reg = ApprovalRegistry::new(Duration::seconds(-1));
        let req = reg.ask("shell", "ls", "ls", RiskLevel::Low).unwrap();
        assert!(reg
            .resolve(req.id.as_str(), ApprovalDecision::Approve)
            .is_err());
        assert!(reg.consume(req.id.as_str(), "shell").is_err());
    }

    #[test]
    fn sweep_marque_les_expirees() {
        let reg = ApprovalRegistry::new(Duration::milliseconds(1));
        let _ = reg.ask("shell", "a", "b", RiskLevel::Low).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert_eq!(reg.sweep().unwrap(), 1);
        assert!(reg.list_pending().unwrap().is_empty());
    }

    #[test]
    fn demande_invalide_rejetee() {
        let reg = ApprovalRegistry::default_ttl();
        assert!(reg.ask("", "x", "y", RiskLevel::Low).is_err());
        assert!(reg.ask("shell", "  ", "y", RiskLevel::Low).is_err());
    }

    #[test]
    fn identifiant_inconnu_introuvable() {
        let reg = ApprovalRegistry::default_ttl();
        assert!(matches!(
            reg.consume("apr-inconnu", "shell"),
            Err(IhError::NotFound(_))
        ));
    }
}
