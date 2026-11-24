//! Sécurité des appels d'outils MCP.
//!
//! Trois responsabilités, toutes indépendantes du transport :
//!
//! 1. **Classification** d'un outil découvert (lecture, écriture, exécution,
//!    destructif, réseau externe) en un [`RiskLevel`] de `ih-security` ;
//! 2. **Politique d'appel** : quand une approbation interactive est exigée et
//!    quand une pré-autorisation explicite (mode sans interaction des
//!    automatisations) peut suffire ;
//! 3. **Empreinte** des paramètres exacts d'un appel, liée à l'approbation
//!    one-shot ([`crate::ApprovalRegistry::ask_with_fingerprint`]) : une
//!    approbation ne vaut jamais pour « un autre appel du même outil ».
//!
//! Les annotations d'un serveur MCP (`readOnlyHint`, `destructiveHint`,
//! `openWorldHint`) ne sont jamais fiables : elles servent de **signal** pour
//! la classification proposée à l'utilisateur, pas de permission. Un outil
//! classé lecture exige quand même une approbation, sauf pré-autorisation
//! explicite du serveur (jamais implicite).

use crate::risk::RiskLevel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Compte du trousseau qui conserve le secret d'un serveur MCP.
pub fn mcp_secret_account(server_id: &str) -> String {
    format!("mcp:{server_id}")
}

/// Catégorie métier d'un outil MCP, miroir de la table des dangers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpToolCategory {
    Read,
    Write,
    Execute,
    Destructive,
    Network,
}

impl McpToolCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Execute => "execute",
            Self::Destructive => "destructive",
            Self::Network => "network",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "execute" => Some(Self::Execute),
            "destructive" => Some(Self::Destructive),
            "network" => Some(Self::Network),
            _ => None,
        }
    }

    pub fn label_fr(self) -> &'static str {
        match self {
            Self::Read => "lecture",
            Self::Write => "écriture",
            Self::Execute => "exécution",
            Self::Destructive => "destructif",
            Self::Network => "réseau externe",
        }
    }

    fn default_risk(self) -> RiskLevel {
        match self {
            Self::Read => RiskLevel::Low,
            Self::Write => RiskLevel::Medium,
            Self::Execute => RiskLevel::High,
            Self::Destructive => RiskLevel::Critical,
            Self::Network => RiskLevel::Medium,
        }
    }
}

/// Signaux fournis par la découverte MCP (`annotations` du serveur).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McpToolHints {
    pub read_only: Option<bool>,
    pub destructive: Option<bool>,
    pub open_world: Option<bool>,
}

/// Classification proposée pour un outil découvert et raisons lisibles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolSafety {
    pub category: McpToolCategory,
    pub risk: RiskLevel,
    /// Pourquoi cette classification (affichée à l'utilisateur).
    pub reasons: Vec<String>,
    /// Tant que l'outil n'est pas explicitement activé par l'utilisateur, il
    /// ne peut pas être appelé du tout.
    pub enabled_by_default: bool,
}

impl McpToolSafety {
    /// Classe un outil d'après son nom, sa description et les annotations du
    /// serveur. Les signaux explicites du serveur sont conservateurs : un
    /// `destructiveHint: true` n'est jamais réduit par un `readOnlyHint`.
    pub fn classify(name: &str, description: &str, hints: McpToolHints) -> Self {
        let mut reasons = Vec::new();

        if hints.destructive == Some(true) {
            reasons.push("le serveur déclare l'outil destructif".to_string());
            return Self {
                category: McpToolCategory::Destructive,
                risk: RiskLevel::Critical,
                reasons,
                enabled_by_default: false,
            };
        }
        if hints.read_only == Some(true) {
            reasons.push("le serveur déclare l'outil en lecture seule".to_string());
            return Self {
                category: McpToolCategory::Read,
                risk: RiskLevel::Low,
                reasons,
                enabled_by_default: false,
            };
        }
        if hints.open_world == Some(true) {
            reasons.push("le serveur déclare un outil ouvert sur le réseau".to_string());
            return Self {
                category: McpToolCategory::Network,
                risk: RiskLevel::Medium,
                reasons,
                enabled_by_default: false,
            };
        }

        let text = format!(
            "{}\n{}",
            name.to_ascii_lowercase(),
            description.to_ascii_lowercase()
        );
        let category = if contains_any(&text, DESTRUCTIVE_WORDS) {
            reasons.push("le nom ou la description évoque une suppression".to_string());
            McpToolCategory::Destructive
        } else if contains_any(&text, EXECUTE_WORDS) {
            reasons.push("le nom ou la description évoque une exécution".to_string());
            McpToolCategory::Execute
        } else if contains_any(&text, NETWORK_WORDS) {
            reasons.push("le nom ou la description évoque un accès réseau".to_string());
            McpToolCategory::Network
        } else if contains_any(&text, WRITE_WORDS) {
            reasons.push("le nom ou la description évoque une modification".to_string());
            McpToolCategory::Write
        } else if contains_any(&text, READ_WORDS) {
            reasons.push("le nom ou la description évoque une lecture".to_string());
            McpToolCategory::Read
        } else {
            reasons.push("classification par défaut, prudente".to_string());
            McpToolCategory::Write
        };
        let risk = category.default_risk();
        Self {
            category,
            risk,
            reasons,
            enabled_by_default: false,
        }
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

const READ_WORDS: &[&str] = &[
    "read",
    "get",
    "list",
    "search",
    "find",
    "query",
    "describe",
    "stat",
    "lire",
    "recherche",
    "liste",
    "inspect",
];
const WRITE_WORDS: &[&str] = &[
    "write", "create", "update", "insert", "append", "add", "set", "put", "patch", "upload",
    "save", "modify", "écris", "crée", "modifie", "remplace", "édite",
];
const EXECUTE_WORDS: &[&str] = &[
    "exec", "run", "invoke", "shell", "command", "spawn", "launch", "exécute", "lance", "commande",
];
const DESTRUCTIVE_WORDS: &[&str] = &[
    "delete", "remove", "destroy", "purge", "wipe", "drop", "truncate", "reset", "supprime",
    "efface", "détruit",
];
const NETWORK_WORDS: &[&str] = &[
    "fetch",
    "http",
    "https",
    "request",
    "download",
    "web",
    "url",
    "send",
    "smtp",
    "télécharge",
    "requête",
];

/// Politique de décision pour un appel d'outil MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCallDecision {
    /// Aucune interaction : autorisé parce qu'explicitement pré-autorisé.
    AutoApproved,
    /// Une approbation interactive à usage unique doit être consommée.
    RequiresApproval,
}

/// Qui déclenche l'appel : l'interface, un chat, une automatisation…
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpCaller {
    Ui,
    Chat,
    Agent,
    Team,
    Automation,
}

impl McpCaller {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ui => "ui",
            Self::Chat => "chat",
            Self::Agent => "agent",
            Self::Team => "team",
            Self::Automation => "automation",
        }
    }
}

/// Décide si un appel peut se faire sans interaction, en fonction du risque et
/// de la pré-autorisation explicitement enregistrée sur le serveur.
///
/// Règle : seule une lecture (`RiskLevel::Low`) peut être pré-autorisée, et
/// seulement si l'utilisateur a explicitement activé le mode sans interaction
/// du serveur. Tout le reste exige une approbation interactive, y compris les
/// automatisations (qui échouent alors en `approval_required` et laissent une
/// trace).
pub fn decide_mcp_call(
    risk: RiskLevel,
    server_allows_auto_approve: bool,
    caller: McpCaller,
) -> McpCallDecision {
    let _ = caller; // La règle ne dépend pas de l'appelant : aucune exception.
    if server_allows_auto_approve && risk == RiskLevel::Low {
        McpCallDecision::AutoApproved
    } else {
        McpCallDecision::RequiresApproval
    }
}

/// Empreinte stable des paramètres exacts d'un appel.
///
/// Le JSON est canonicalisé (clés d'objets triées récursivement) avant
/// SHA-256 ; deux appels ne partagent donc l'empreinte que s'ils sont
/// sémantiquement identiques. L'empreinte sert d'engagement : l'approbation
/// liée à cette empreinte ne peut pas être rejouée pour d'autres arguments.
pub fn mcp_call_fingerprint(server_id: &str, tool: &str, arguments: &Value) -> String {
    let canonical = canonical_json(arguments);
    let mut hasher = Sha256::new();
    hasher.update(server_id.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(tool.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(canonical.as_bytes());
    hex::encode(hasher.finalize())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical_json(&map[*key]));
            }
            out.push('}');
            out
        }
        Value::Array(items) => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json(item));
            }
            out.push(']');
            out
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compte_trousseau_par_serveur() {
        assert_eq!(mcp_secret_account("srv-1"), "mcp:srv-1");
    }

    #[test]
    fn classifications_par_annotations_explicites() {
        let read = McpToolSafety::classify(
            "anything",
            "",
            McpToolHints {
                read_only: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(read.category, McpToolCategory::Read);
        assert_eq!(read.risk, RiskLevel::Low);

        let destructive = McpToolSafety::classify(
            "anything",
            "",
            McpToolHints {
                destructive: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(destructive.category, McpToolCategory::Destructive);
        assert_eq!(destructive.risk, RiskLevel::Critical);

        let open = McpToolSafety::classify(
            "anything",
            "",
            McpToolHints {
                open_world: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(open.category, McpToolCategory::Network);
        assert_eq!(open.risk, RiskLevel::Medium);
    }

    #[test]
    fn destructif_l_emporte_sur_lecture() {
        let tool = McpToolSafety::classify(
            "read_file",
            "",
            McpToolHints {
                read_only: Some(true),
                destructive: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(tool.category, McpToolCategory::Destructive);
    }

    #[test]
    fn classifications_par_noms_et_descriptions() {
        let cases = [
            ("get_weather", "", McpToolCategory::Read),
            ("delete_record", "", McpToolCategory::Destructive),
            ("run_script", "", McpToolCategory::Execute),
            ("fetch_url", "", McpToolCategory::Network),
            ("save_note", "", McpToolCategory::Write),
            ("opaque_operation", "", McpToolCategory::Write), // prudent
        ];
        for (name, description, expected) in cases {
            let safety = McpToolSafety::classify(name, description, McpToolHints::default());
            assert_eq!(safety.category, expected, "outil {name}");
            assert!(!safety.reasons.is_empty());
        }
    }

    #[test]
    fn aucun_outil_n_est_active_par_defaut() {
        let safety = McpToolSafety::classify("get_x", "", McpToolHints::default());
        assert!(!safety.enabled_by_default);
    }

    #[test]
    fn auto_approbation_uniquement_pour_les_lectures() {
        assert_eq!(
            decide_mcp_call(RiskLevel::Low, true, McpCaller::Automation),
            McpCallDecision::AutoApproved
        );
        // La pré-autorisation ne suffit jamais pour une écriture ou plus.
        assert_eq!(
            decide_mcp_call(RiskLevel::Medium, true, McpCaller::Automation),
            McpCallDecision::RequiresApproval
        );
        assert_eq!(
            decide_mcp_call(RiskLevel::High, true, McpCaller::Ui),
            McpCallDecision::RequiresApproval
        );
        // Pas de pré-autorisation : approbation interactive, quel que soit l'appelant.
        for caller in [
            McpCaller::Ui,
            McpCaller::Chat,
            McpCaller::Agent,
            McpCaller::Team,
            McpCaller::Automation,
        ] {
            assert_eq!(
                decide_mcp_call(RiskLevel::Low, false, caller),
                McpCallDecision::RequiresApproval
            );
        }
    }

    #[test]
    fn empreinte_stable_et_sensible_aux_parametres() {
        let a = mcp_call_fingerprint("srv", "tool", &json!({"a": 1, "b": [1, 2]}));
        let b = mcp_call_fingerprint("srv", "tool", &json!({"b": [1, 2], "a": 1}));
        assert_eq!(a, b, "l'ordre des clés ne doit pas changer l'empreinte");

        let c = mcp_call_fingerprint("srv", "tool", &json!({"a": 2, "b": [1, 2]}));
        assert_ne!(a, c);

        let d = mcp_call_fingerprint("autre", "tool", &json!({"a": 1, "b": [1, 2]}));
        assert_ne!(a, d);

        let e = mcp_call_fingerprint("srv", "autre", &json!({"a": 1, "b": [1, 2]}));
        assert_ne!(a, e);
    }

    #[test]
    fn empreinte_identifie_la_structure() {
        // {"a": {"x":1}} et {"a": 1} ne doivent pas entrer en collision.
        let f1 = mcp_call_fingerprint("s", "t", &json!({"a": {"x": 1}}));
        let f2 = mcp_call_fingerprint("s", "t", &json!({"a": 1}));
        assert_ne!(f1, f2);
        assert_eq!(f1.len(), 64);
    }
}
