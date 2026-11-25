//! Catalogue d'outils MCP exposés au chat en tool calling natif.
//!
//! Un serveur MCP peut déclarer des noms qui ne sont pas des identifiants
//! d'outil valides côté fournisseur (points, espaces, tirets multiples) ou
//! entrer en collision après normalisation. Le catalogue expose donc un nom
//! sûr (≤ 64 caractères, `[a-zA-Z0-9_-]`) et conserve le couple réel
//! (serveur, outil) côté noyau : le modèle ne voit jamais d'URL, de jeton ni
//! d'en-tête.
//!
//! Seuls les serveurs **activés et connectés** et les outils **activés par
//! l'utilisateur** sont exposés. Un outil dont le schéma d'entrée n'est pas un
//! objet JSON est écarté : le tool calling natif n'accepte pas d'autre forme.

use crate::state::AppState;
use ih_core::IhResult;
use ih_providers::ToolDefinition;
use ih_store::{McpServerRecord, McpToolRecord};
use serde::Serialize;

use std::collections::BTreeMap;

/// Longueur maximale d'un nom d'outil natif (`^[a-zA-Z0-9_-]{1,64}$`).
pub const MAX_TOOL_NAME: usize = 64;
const MAX_TOOLS: usize = 24;
const MAX_DESCRIPTION_CHARS: usize = 1024;
const MAX_SCHEMA_BYTES: usize = 8 * 1024;
const MAX_TOTAL_SCHEMA_BYTES: usize = 32 * 1024;

/// Cible réelle derrière un nom d'outil exposé au modèle.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatToolTarget {
    pub exposed_name: String,
    pub server_id: String,
    pub server_name: String,
    pub tool: String,
    pub risk: String,
    pub category: String,
}

/// Catalogue prêt pour une requête de chat.
#[derive(Debug, Clone, Default)]
pub struct ChatToolCatalog {
    pub definitions: Vec<ToolDefinition>,
    pub targets: BTreeMap<String, ChatToolTarget>,
}

impl ChatToolCatalog {
    /// Cible réelle pour un nom exposé (jamais l'inverse).
    pub fn target(&self, exposed_name: &str) -> Option<&ChatToolTarget> {
        self.targets.get(exposed_name)
    }
}

/// Vue IPC d'un outil natif : sans secret, sans URL, sans en-tête.
#[derive(Debug, Serialize)]
pub struct ChatToolView {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub server_id: String,
    pub server_name: String,
    pub tool: String,
    pub risk: String,
    pub category: String,
}

/// Décrit les outils réellement proposables au modèle.
///
/// Lit uniquement la base (aucune connexion, aucun trousseau) : un serveur
/// déconnecté ou en erreur n'expose rien, même s'il reste enregistré.
pub fn chat_tool_catalog(state: &AppState) -> IhResult<ChatToolCatalog> {
    let servers = state.store.list_mcp_servers()?;
    let mut snapshots: Vec<(McpServerRecord, Vec<McpToolRecord>)> = Vec::new();
    for server in servers {
        let tools = state
            .store
            .list_mcp_tools(Some(&server.id))
            .unwrap_or_default();
        snapshots.push((server, tools));
    }
    Ok(catalog_from(&snapshots))
}

/// Construit le catalogue à partir d'instantanés (serveur, outils).
pub fn catalog_from(snapshots: &[(McpServerRecord, Vec<McpToolRecord>)]) -> ChatToolCatalog {
    let mut catalog = ChatToolCatalog::default();
    let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut schema_budget = MAX_TOTAL_SCHEMA_BYTES;

    for (server, tools) in snapshots {
        if !server.enabled || server.last_status != "connected" {
            continue;
        }
        let mut ordered: Vec<&McpToolRecord> = tools.iter().filter(|tool| tool.enabled).collect();
        ordered.sort_by(|left, right| left.name.cmp(&right.name));
        for tool in ordered {
            if catalog.definitions.len() >= MAX_TOOLS {
                return catalog;
            }
            let parameters = match valid_parameters(&tool.input_schema, schema_budget) {
                Some(parameters) => parameters,
                None => continue,
            };
            let exposed_name = unique_entry_name(&server.id, &tool.name, &mut used);
            let description = describe(server, tool);
            schema_budget = schema_budget.saturating_sub(
                serde_json::to_string(&parameters)
                    .map(|text| text.len())
                    .unwrap_or(0),
            );
            catalog.definitions.push(ToolDefinition {
                name: exposed_name.clone(),
                description,
                parameters,
            });
            catalog.targets.insert(
                exposed_name.clone(),
                ChatToolTarget {
                    exposed_name: exposed_name.clone(),
                    server_id: server.id.clone(),
                    server_name: server.name.clone(),
                    tool: tool.name.clone(),
                    risk: tool.risk.clone(),
                    category: tool.category.clone(),
                },
            );
        }
    }
    catalog
}

/// Normalise et garantit l'unicité du nom exposé.
///
/// Base : `{serveur}__{outil}`, caractères hors `[a-zA-Z0-9_-]` remplacés par
/// `_`. Un nom trop long garde ses premiers caractères et un hachage
/// déterministe du nom complet ; une collision obtient un suffixe numérique.
fn unique_entry_name(
    server_id: &str,
    tool_name: &str,
    used: &mut std::collections::BTreeSet<String>,
) -> String {
    let base = sanitize(&format!("{server_id}__{tool_name}"));
    let mut candidate = if base.len() <= MAX_TOOL_NAME {
        base.clone()
    } else {
        shorten(&base)
    };
    if !used.contains(&candidate) {
        used.insert(candidate.clone());
        return candidate;
    }
    let mut counter = 2usize;
    loop {
        let suffix = format!("_{counter}");
        let keep = MAX_TOOL_NAME - suffix.len();
        let stem: String = candidate.chars().take(keep).collect();
        let attempt = format!("{}{suffix}", stem);
        if !used.contains(&attempt) {
            used.insert(attempt.clone());
            return attempt;
        }
        counter += 1;
        if counter > 4096 {
            // Impossibles en pratique : on repart d'un hachage complet.
            candidate = shorten(&format!("{base}_{counter}"));
            counter = 2;
        }
    }
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Tronque en gardant la place d'un hachage FNV-1a stable (8 caractères).
fn shorten(base: &str) -> String {
    let suffix = format!("_{}", fnv1a_hex(base));
    let keep = MAX_TOOL_NAME - suffix.len();
    let stem: String = base.chars().take(keep).collect();
    format!("{stem}{suffix}")
}

fn fnv1a_hex(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

fn describe(server: &McpServerRecord, tool: &McpToolRecord) -> String {
    let text = format!(
        "{} · risque {} · {}",
        server.name.trim(),
        tool.risk,
        tool.description.trim()
    );
    text.chars().take(MAX_DESCRIPTION_CHARS).collect()
}

/// Schéma d'entrée utilisable par le tool calling (objet JSON seulement).
fn valid_parameters(schema: &serde_json::Value, available: usize) -> Option<serde_json::Value> {
    let object = schema.as_object()?;
    let params = if object.get("type").and_then(|value| value.as_str()) == Some("object") {
        serde_json::Value::Object(object.clone())
    } else if object.contains_key("properties") {
        // Schéma sans `type` explicite : on le déclare objet sans l'inventer.
        let mut cloned = object.clone();
        cloned.insert("type".into(), serde_json::Value::String("object".into()));
        serde_json::Value::Object(cloned)
    } else {
        return None;
    };
    let size = serde_json::to_string(&params)
        .map(|text| text.len())
        .unwrap_or(usize::MAX);
    if size > MAX_SCHEMA_BYTES || size > available {
        return None;
    }
    Some(params)
}

/// Vue IPC du catalogue courant (aucun secret, aucune URL).
#[tauri::command]
pub fn chat_tools_list(state: tauri::State<'_, AppState>) -> IhResult<Vec<ChatToolView>> {
    let catalog = chat_tool_catalog(&state)?;
    Ok(catalog
        .definitions
        .iter()
        .filter_map(|definition| {
            catalog.target(&definition.name).map(|target| ChatToolView {
                name: definition.name.clone(),
                description: definition.description.clone(),
                parameters: definition.parameters.clone(),
                server_id: target.server_id.clone(),
                server_name: target.server_name.clone(),
                tool: target.tool.clone(),
                risk: target.risk.clone(),
                category: target.category.clone(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn server(id: &str, status: &str, enabled: bool) -> McpServerRecord {
        McpServerRecord {
            id: id.into(),
            name: format!("Serveur {id}"),
            url: "http://127.0.0.1:9/mcp".into(),
            transport: "streamable_http".into(),
            headers: Default::default(),
            timeout_secs: 5,
            enabled,
            allow_auto_approve: false,
            has_secret: true,
            last_status: status.into(),
            last_error: None,
            last_connected_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn tool(name: &str, enabled: bool) -> McpToolRecord {
        McpToolRecord {
            server_id: "srv".into(),
            name: name.into(),
            description: "outil de test".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
            }),
            output_schema: None,
            category: "read".into(),
            risk: "low".into(),
            enabled,
            discovered_at: Utc::now(),
        }
    }

    #[test]
    fn seuls_les_serveurs_connectes_et_activés_exposent_des_outils() {
        let catalog = catalog_from(&[
            (server("a", "connected", true), vec![tool("echo", true)]),
            (server("b", "disconnected", true), vec![tool("echo", true)]),
            (server("c", "error", true), vec![tool("echo", true)]),
            (server("d", "connected", false), vec![tool("echo", true)]),
        ]);
        let names: Vec<&str> = catalog
            .definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();
        assert_eq!(names, vec!["a__echo"]);
    }

    #[test]
    fn les_outils_désactivés_n_sont_jamais_exposés() {
        let catalog = catalog_from(&[(
            server("a", "connected", true),
            vec![tool("echo", true), tool("cache", false)],
        )]);
        assert_eq!(catalog.definitions.len(), 1);
        assert_eq!(catalog.definitions[0].name, "a__echo");
    }

    #[test]
    fn noms_invalides_et_collisions_sont_normalisés() {
        let catalog = catalog_from(&[
            (
                server("a", "connected", true),
                vec![tool("do.thing", true), tool("do thing", true)],
            ),
            (server("b", "connected", true), vec![tool("echo", true)]),
        ]);
        let mut names: Vec<String> = catalog
            .definitions
            .iter()
            .map(|definition| definition.name.clone())
            .collect();
        names.sort();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"a__do_thing".to_string()));
        assert!(names.contains(&"a__do_thing_2".to_string()));
        assert!(names.contains(&"b__echo".to_string()));
        for name in &names {
            assert!(name.len() <= MAX_TOOL_NAME, "nom trop long : {name}");
        }
    }

    #[test]
    fn un_nom_trop_long_est_tronqué_avec_hachage_stable() {
        let long = "outil".repeat(40);
        let catalog = catalog_from(&[(
            server("serveur", "connected", true),
            vec![tool(&long, true)],
        )]);
        let name = &catalog.definitions[0].name;
        assert!(name.len() <= MAX_TOOL_NAME);
        let again = catalog_from(&[(
            server("serveur", "connected", true),
            vec![tool(&long, true)],
        )]);
        assert_eq!(name, &again.definitions[0].name);
    }

    #[test]
    fn schéma_non_objet_écarté() {
        let mut bad = tool("raw", true);
        bad.input_schema = serde_json::json!({"type": "string"});
        let mut absent = tool("sans_type", true);
        absent.input_schema = serde_json::json!({"properties": {"a": {"type": "string"}}});
        let catalog = catalog_from(&[(server("a", "connected", true), vec![bad, absent])]);
        assert_eq!(catalog.definitions.len(), 1);
        assert_eq!(catalog.definitions[0].parameters["type"], "object");
    }

    #[test]
    fn la_cible_réelle_est_résoluble_et_sans_secret() {
        let catalog = catalog_from(&[(server("a", "connected", true), vec![tool("echo", true)])]);
        let target = catalog.target("a__echo").expect("cible");
        assert_eq!(target.server_id, "a");
        assert_eq!(target.tool, "echo");
        assert_eq!(target.risk, "low");
        let definition = &catalog.definitions[0];
        assert!(!definition.description.contains("127.0.0.1"));
        assert!(!definition.description.contains("http"));
    }

    #[test]
    fn le_budget_d_outils_est_borné() {
        let many: Vec<McpToolRecord> = (0..40).map(|i| tool(&format!("outil_{i}"), true)).collect();
        let catalog = catalog_from(&[(server("a", "connected", true), many)]);
        assert_eq!(catalog.definitions.len(), MAX_TOOLS);
    }
}
