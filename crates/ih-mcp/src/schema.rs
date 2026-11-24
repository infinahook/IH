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

/// Taille maximale des arguments acceptés pour exécution (256 Kio).
pub const MAX_ARGUMENTS_BYTES: usize = 256 * 1024;

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

/// Vérifie que les arguments d'un appel respectent les contraintes
/// structurelles du schéma : objet, propriétés typées simple, champs requis,
/// `additionalProperties: false` respecté.
///
/// La validation reste volontairement partielle (sous-ensemble JSON Schema) :
/// elle attrape les erreurs évidentes avant tout appel réseau, sans prétendre
/// remplacer un validateur complet.
pub fn validate_arguments(schema: &Value, arguments: &Value) -> IhResult<()> {
    let size = serde_json::to_string(arguments)
        .map(|text| text.len())
        .unwrap_or(usize::MAX);
    if size > MAX_ARGUMENTS_BYTES {
        return Err(IhError::Invalid(format!(
            "arguments MCP trop volumineux ({size} octets, limite {MAX_ARGUMENTS_BYTES})"
        )));
    }
    let args = arguments.as_object().ok_or_else(|| {
        IhError::Invalid("les arguments d'un outil MCP doivent être un objet JSON".into())
    })?;
    let properties = schema.get("properties").and_then(Value::as_object);

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for entry in required {
            let name = entry.as_str().unwrap_or_default();
            if !args.contains_key(name) {
                return Err(IhError::Invalid(format!(
                    "argument requis manquant : {name}"
                )));
            }
        }
    }

    if let Some(properties) = properties {
        for (name, value) in args {
            let Some(property) = properties.get(name) else {
                continue;
            };
            let Some(kind) = property.get("type").and_then(Value::as_str) else {
                continue;
            };
            if value.is_null() {
                continue;
            }
            if !matches_type(kind, value) {
                return Err(IhError::Invalid(format!(
                    "argument « {name} » de type {kind} attendu, reçu : {}",
                    json_type(value)
                )));
            }
        }
    }

    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        if let Some(properties) = properties {
            for key in args.keys() {
                if !properties.contains_key(key) {
                    return Err(IhError::Invalid(format!(
                        "argument inattendu refusé par le schéma : {key}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn matches_type(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value.as_f64().map(|n| n.fract() == 0.0).unwrap_or(false)
        }
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
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
    fn arguments_valides_acceptes() {
        let schema = json!({
            "type": "object",
            "properties": {"a": {"type": "integer"}, "b": {"type": "string"}},
            "required": ["a"]
        });
        validate_arguments(&schema, &json!({"a": 3, "b": "x"})).unwrap();
        validate_arguments(&schema, &json!({"a": 3})).unwrap();
    }

    #[test]
    fn argument_requis_manquant_refuse() {
        let schema =
            json!({"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]});
        let error = validate_arguments(&schema, &json!({})).unwrap_err();
        assert!(error.to_string().contains("requis"), "{error}");
    }

    #[test]
    fn type_d_argument_invalide_refuse() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "integer"}}});
        let error = validate_arguments(&schema, &json!({"a": "trois"})).unwrap_err();
        assert!(error.to_string().contains("integer"), "{error}");
        validate_arguments(&schema, &json!({"a": 3.0})).unwrap();
        validate_arguments(&schema, &json!({"a": null})).unwrap();
    }

    #[test]
    fn arguments_trop_volumineux_refuses() {
        let schema = json!({"type": "object", "properties": {"data": {"type": "string"}}});
        let big = json!({"data": "x".repeat(super::MAX_ARGUMENTS_BYTES + 10)});
        let error = validate_arguments(&schema, &big).unwrap_err();
        assert!(error.to_string().contains("volumineux"), "{error}");
    }

    #[test]
    fn arguments_non_objets_refuses() {
        let schema = json!({"type": "object"});
        assert!(validate_arguments(&schema, &json!([1, 2])).is_err());
    }

    #[test]
    fn additional_properties_false_applique() {
        let schema = json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "additionalProperties": false
        });
        assert!(validate_arguments(&schema, &json!({"a": "x", "b": 1})).is_err());
        validate_arguments(&schema, &json!({"a": "x"})).unwrap();
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
