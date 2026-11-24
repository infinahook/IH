//! Validation structurelle des schémas d'entrée d'outils MCP.
//!
//! Un serveur MCP est une source non fiable : son `inputSchema` peut être
//! malformé, énorme ou piégé. IH ne valide pas le JSON Schema complet (les
//! serveurs utilisent des sous-ensembles variables), mais il **refuse** tout
//! schéma qui ne peut pas décrire un objet d'arguments ou qui dépasse les
//! bornes ci-dessous. Un schéma refusé ne casse pas la connexion : l'outil est
//! simplement exclu du catalogue, avec la raison.

use ih_core::{IhError, IhResult};
use serde_json::Value;

/// Taille maximale d'un schéma d'entrée (512 Kio).
pub const MAX_SCHEMA_BYTES: usize = 512 * 1024;
/// Profondeur maximale d'imbrication du schéma.
pub const MAX_SCHEMA_DEPTH: usize = 16;
/// Nombre maximal de nœuds parcourus dans le schéma.
pub const MAX_SCHEMA_NODES: usize = 20_000;

/// Valide le schéma d'entrée d'un outil MCP.
pub fn validate_tool_schema(schema: &Value) -> IhResult<()> {
    if !schema.is_object() {
        return Err(IhError::Invalid(
            "le schéma d'entrée d'un outil MCP doit être un objet JSON".into(),
        ));
    }
    if let Some(kind) = schema.get("type") {
        if kind.as_str() != Some("object") {
            return Err(IhError::Invalid(format!(
                "le schéma d'entrée d'un outil MCP doit être de type object, reçu : {kind}"
            )));
        }
    }
    if let Some(properties) = schema.get("properties") {
        let properties = properties.as_object().ok_or_else(|| {
            IhError::Invalid("« properties » du schéma MCP doit être un objet".into())
        })?;
        for (name, property) in properties {
            if !property.is_object() {
                return Err(IhError::Invalid(format!(
                    "la propriété « {name} » du schéma MCP doit être un objet JSON"
                )));
            }
        }
        if let Some(required) = schema.get("required") {
            let required = required.as_array().ok_or_else(|| {
                IhError::Invalid("« required » du schéma MCP doit être un tableau".into())
            })?;
            for entry in required {
                let name = entry.as_str().ok_or_else(|| {
                    IhError::Invalid("« required » du schéma MCP doit contenir des chaînes".into())
                })?;
                if !properties.contains_key(name) {
                    return Err(IhError::Invalid(format!(
                        "« required » cite « {name} » absent de « properties »"
                    )));
                }
            }
        }
    }
    check_bounds(schema, 0)?;
    Ok(())
}

fn check_bounds(value: &Value, _depth: usize) -> IhResult<()> {
    let mut nodes = 0usize;
    let mut stack: Vec<(&Value, usize)> = vec![(value, 1)];
    while let Some((current, depth)) = stack.pop() {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(IhError::Invalid(
                "schéma MCP trop profond (limite d'imbrication)".into(),
            ));
        }
        nodes += 1;
        if nodes > MAX_SCHEMA_NODES {
            return Err(IhError::Invalid(
                "schéma MCP trop volumineux (limite de nœuds)".into(),
            ));
        }
        match current {
            Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                    // Seules les références internes au document sont tolérées :
                    // une $ref distante ferait charger une ressource arbitraire.
                    if !reference.starts_with('#') {
                        return Err(IhError::Invalid(format!(
                            "référence « {reference} » refusée dans un schéma MCP (references locales uniquement)"
                        )));
                    }
                }
                for nested in map.values() {
                    if nested.is_object() || nested.is_array() {
                        stack.push((nested, depth + 1));
                    }
                }
            }
            Value::Array(items) => {
                for nested in items {
                    if nested.is_object() || nested.is_array() {
                        stack.push((nested, depth + 1));
                    }
                }
            }
            _ => {}
        }
    }
    let size = serde_json::to_string(value)
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if size > MAX_SCHEMA_BYTES {
        return Err(IhError::Invalid(format!(
            "schéma MCP trop volumineux ({size} octets, limite {MAX_SCHEMA_BYTES})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_valide_accepte() {
        validate_tool_schema(&json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "count": {"type": "integer", "minimum": 0}
            },
            "required": ["path"]
        }))
        .unwrap();
    }

    #[test]
    fn schema_sans_type_ni_properties_accepte() {
        validate_tool_schema(&json!({"additionalProperties": false})).unwrap();
        validate_tool_schema(&json!({})).unwrap();
    }

    #[test]
    fn schema_non_objet_refuse() {
        assert!(validate_tool_schema(&json!([1, 2, 3])).is_err());
        assert!(validate_tool_schema(&json!("string")).is_err());
    }

    #[test]
    fn type_non_objet_refuse() {
        assert!(validate_tool_schema(&json!({"type": "array"})).is_err());
        assert!(validate_tool_schema(&json!({"type": "string"})).is_err());
    }

    #[test]
    fn properties_malformees_refusees() {
        assert!(validate_tool_schema(&json!({"properties": "oops"})).is_err());
        assert!(validate_tool_schema(&json!({"properties": {"a": 3}})).is_err());
    }

    #[test]
    fn required_incoherent_refuse() {
        assert!(validate_tool_schema(&json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "required": ["b"]
        }))
        .is_err());
    }

    #[test]
    fn reference_distante_refusee() {
        assert!(validate_tool_schema(&json!({
            "type": "object",
            "properties": {"x": {"$ref": "https://evil.example/schema.json"}}
        }))
        .is_err());
        validate_tool_schema(&json!({
            "type": "object",
            "properties": {"x": {"$ref": "#/definitions/ok"}},
            "definitions": {"ok": {"type": "string"}}
        }))
        .unwrap();
    }

    #[test]
    fn schema_trop_profond_refuse() {
        let mut schema = json!({"type": "object"});
        let mut current = &mut schema;
        for _ in 0..40 {
            current["properties"] = json!({"n": {"type": "object"}});
            current = current.get_mut("properties").unwrap().get_mut("n").unwrap();
        }
        assert!(validate_tool_schema(&schema).is_err());
    }

    #[test]
    fn schema_trop_volumineux_refuse() {
        let big = "x".repeat(MAX_SCHEMA_BYTES + 10);
        let schema = json!({
            "type": "object",
            "properties": {"p": {"type": "string", "description": big}}
        });
        assert!(validate_tool_schema(&schema).is_err());
    }
}
