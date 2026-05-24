use chrono::Utc;
use fake::faker::internet::en::SafeEmail;
use fake::faker::name::en::Name;
use fake::Fake;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum GeneratorConfig {
    Uuid,
    #[serde(rename = "date-time")]
    DateTime {
        #[serde(default)]
        format: Option<String>,
    },
    String {
        #[serde(rename = "minLength", default)]
        min_length: Option<usize>,
        #[serde(rename = "maxLength", default)]
        max_length: Option<usize>,
    },
    Int {
        #[serde(default)]
        min: Option<i64>,
        #[serde(default)]
        max: Option<i64>,
    },
    Bool,
    Email,
    Name,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenBlock {
    #[serde(flatten)]
    pub config: GeneratorConfig,
}

pub fn validate_generator(config: &GeneratorConfig, field_path: &str) -> Result<(), String> {
    match config {
        GeneratorConfig::Uuid => Ok(()),
        GeneratorConfig::DateTime { format } => {
            if let Some(fmt) = format {
                let supported = ["rfc3339", "iso8601"];
                if !supported.contains(&fmt.as_str()) {
                    return Err(format!(
                        "generation_error: unsupported date-time format '{}' at '{}'; supported: rfc3339, iso8601",
                        fmt, field_path
                    ));
                }
            }
            Ok(())
        }
        GeneratorConfig::String { min_length, max_length } => {
            let min = min_length.unwrap_or(1);
            let max = max_length.unwrap_or(32);
            if min > max {
                return Err(format!(
                    "generation_error: minLength ({}) > maxLength ({}) at '{}'",
                    min, max, field_path
                ));
            }
            Ok(())
        }
        GeneratorConfig::Int { min, max } => {
            if let (Some(mn), Some(mx)) = (min, max) {
                if mn > mx {
                    return Err(format!(
                        "generation_error: min ({}) > max ({}) at '{}'",
                        mn, mx, field_path
                    ));
                }
            }
            Ok(())
        }
        GeneratorConfig::Bool => Ok(()),
        GeneratorConfig::Email => Ok(()),
        GeneratorConfig::Name => Ok(()),
    }
}

pub fn generate(config: &GeneratorConfig) -> Value {
    let mut rng = rand::thread_rng();
    match config {
        GeneratorConfig::Uuid => Value::String(uuid::Uuid::new_v4().to_string()),
        GeneratorConfig::DateTime { format } => {
            let now = Utc::now();
            let formatted = match format.as_deref() {
                Some("iso8601") => now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                _ => now.to_rfc3339(),
            };
            Value::String(formatted)
        }
        GeneratorConfig::String { min_length, max_length } => {
            let min = min_length.unwrap_or(1);
            let max = max_length.unwrap_or(32);
            let len = if min == max { min } else { rng.gen_range(min..=max) };
            let chars: std::string::String = (0..len)
                .map(|_| {
                    let idx = rng.gen_range(0..62usize);
                    match idx {
                        0..=25 => (b'a' + idx as u8) as char,
                        26..=51 => (b'A' + (idx - 26) as u8) as char,
                        _ => (b'0' + (idx - 52) as u8) as char,
                    }
                })
                .collect();
            Value::String(chars)
        }
        GeneratorConfig::Int { min, max } => {
            let mn = min.unwrap_or(i64::MIN / 2);
            let mx = max.unwrap_or(i64::MAX / 2);
            let v = if mn == mx { mn } else { rng.gen_range(mn..=mx) };
            Value::Number(v.into())
        }
        GeneratorConfig::Bool => Value::Bool(rng.gen_bool(0.5)),
        GeneratorConfig::Email => Value::String(SafeEmail().fake::<std::string::String>()),
        GeneratorConfig::Name => Value::String(Name().fake::<std::string::String>()),
    }
}

/// Walk a `serde_json::Value` tree and resolve any `{"gen": {...}}` nodes,
/// returning (resolved_value, errors).
pub fn resolve_gen_values(value: &Value, path: &str) -> (Value, Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(gen_val) = map.get("gen") {
                if map.len() == 1 {
                    match serde_json::from_value::<GeneratorConfig>(gen_val.clone()) {
                        Ok(cfg) => {
                            let mut errors = Vec::new();
                            if let Err(e) = validate_generator(&cfg, path) {
                                errors.push(e);
                                return (Value::Null, errors);
                            }
                            return (generate(&cfg), Vec::new());
                        }
                        Err(e) => {
                            return (
                                Value::Null,
                                vec![format!(
                                    "generation_error: invalid gen config at '{}': {}",
                                    path, e
                                )],
                            );
                        }
                    }
                }
            }
            let mut out = serde_json::Map::new();
            let mut all_errors = Vec::new();
            for (k, v) in map {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", path, k)
                };
                let (resolved, errs) = resolve_gen_values(v, &child_path);
                out.insert(k.clone(), resolved);
                all_errors.extend(errs);
            }
            (Value::Object(out), all_errors)
        }
        Value::Array(arr) => {
            let mut out = Vec::new();
            let mut all_errors = Vec::new();
            for (i, v) in arr.iter().enumerate() {
                let child_path = format!("{}[{}]", path, i);
                let (resolved, errs) = resolve_gen_values(v, &child_path);
                out.push(resolved);
                all_errors.extend(errs);
            }
            (Value::Array(out), all_errors)
        }
        other => (other.clone(), Vec::new()),
    }
}

/// Resolve all `gen` blocks in a BTreeMap of variable values.
pub fn resolve_gen_variables(
    variables: &std::collections::BTreeMap<std::string::String, Value>,
) -> Result<std::collections::BTreeMap<std::string::String, Value>, Vec<String>> {
    let mut out = std::collections::BTreeMap::new();
    let mut all_errors = Vec::new();
    for (k, v) in variables {
        let (resolved, errs) = resolve_gen_values(v, k);
        out.insert(k.clone(), resolved);
        all_errors.extend(errs);
    }
    if all_errors.is_empty() {
        Ok(out)
    } else {
        Err(all_errors)
    }
}

/// Validate `gen` blocks in a variables map without generating values.
pub fn validate_gen_in_variables(
    variables: &std::collections::BTreeMap<std::string::String, Value>,
    file_path: &str,
) -> Vec<String> {
    let mut errors = Vec::new();
    for (k, v) in variables {
        errors.extend(validate_gen_in_value(v, k, file_path));
    }
    errors
}

/// Validate `gen` blocks inside a JSON value tree without generating values.
pub fn validate_gen_in_value(value: &Value, path: &str, file_path: &str) -> Vec<String> {
    match value {
        Value::Object(map) => {
            if let Some(gen_val) = map.get("gen") {
                if map.len() == 1 {
                    match serde_json::from_value::<GeneratorConfig>(gen_val.clone()) {
                        Ok(cfg) => {
                            if let Err(e) = validate_generator(&cfg, path) {
                                return vec![format!("{} in {}", e, file_path)];
                            }
                            return Vec::new();
                        }
                        Err(e) => {
                            return vec![format!(
                                "generation_error: invalid gen config at '{}' in {}: {}",
                                path, file_path, e
                            )];
                        }
                    }
                }
            }
            let mut all_errors = Vec::new();
            for (k, v) in map {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", path, k)
                };
                all_errors.extend(validate_gen_in_value(v, &child_path, file_path));
            }
            all_errors
        }
        Value::Array(arr) => {
            let mut all_errors = Vec::new();
            for (i, v) in arr.iter().enumerate() {
                let child_path = format!("{}[{}]", path, i);
                all_errors.extend(validate_gen_in_value(v, &child_path, file_path));
            }
            all_errors
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_uuid_is_valid() {
        let val = generate(&GeneratorConfig::Uuid);
        let s = val.as_str().unwrap();
        assert_eq!(s.len(), 36);
        assert!(s.contains('-'));
    }

    #[test]
    fn generate_datetime_rfc3339() {
        let val = generate(&GeneratorConfig::DateTime { format: None });
        let s = val.as_str().unwrap();
        assert!(s.contains('T'), "expected RFC3339 date, got: {}", s);
    }

    #[test]
    fn generate_datetime_iso8601() {
        let val = generate(&GeneratorConfig::DateTime {
            format: Some("iso8601".to_string()),
        });
        let s = val.as_str().unwrap();
        assert!(s.ends_with('Z'), "expected iso8601 with Z suffix, got: {}", s);
    }

    #[test]
    fn generate_string_length_in_range() {
        let val = generate(&GeneratorConfig::String {
            min_length: Some(8),
            max_length: Some(16),
        });
        let s = val.as_str().unwrap();
        assert!(s.len() >= 8 && s.len() <= 16, "len={}", s.len());
    }

    #[test]
    fn generate_int_in_range() {
        let val = generate(&GeneratorConfig::Int {
            min: Some(18),
            max: Some(60),
        });
        let n = val.as_i64().unwrap();
        assert!(n >= 18 && n <= 60, "n={}", n);
    }

    #[test]
    fn generate_bool_is_bool() {
        let val = generate(&GeneratorConfig::Bool);
        assert!(val.is_boolean());
    }

    #[test]
    fn generate_email_contains_at() {
        let val = generate(&GeneratorConfig::Email);
        assert!(val.as_str().unwrap().contains('@'));
    }

    #[test]
    fn generate_name_is_nonempty() {
        let val = generate(&GeneratorConfig::Name);
        assert!(!val.as_str().unwrap().is_empty());
    }

    #[test]
    fn validate_int_min_gt_max_errors() {
        let result = validate_generator(
            &GeneratorConfig::Int {
                min: Some(100),
                max: Some(10),
            },
            "age",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("generation_error"));
    }

    #[test]
    fn validate_string_minlength_gt_maxlength_errors() {
        let result = validate_generator(
            &GeneratorConfig::String {
                min_length: Some(20),
                max_length: Some(5),
            },
            "name",
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_datetime_unsupported_format_errors() {
        let result = validate_generator(
            &GeneratorConfig::DateTime {
                format: Some("unix".to_string()),
            },
            "ts",
        );
        assert!(result.is_err());
    }

    #[test]
    fn resolve_gen_values_variable_uuid() {
        let v: Value = serde_json::json!({ "gen": { "type": "uuid" } });
        let (resolved, errors) = resolve_gen_values(&v, "userId");
        assert!(errors.is_empty());
        let s = resolved.as_str().unwrap();
        assert_eq!(s.len(), 36);
    }

    #[test]
    fn resolve_gen_values_nested_object() {
        let v: Value = serde_json::json!({
            "id": { "gen": { "type": "uuid" } },
            "age": { "gen": { "type": "int", "min": 18, "max": 60 } }
        });
        let (resolved, errors) = resolve_gen_values(&v, "body");
        assert!(errors.is_empty());
        let obj = resolved.as_object().unwrap();
        assert!(obj["id"].as_str().unwrap().contains('-'));
        let age = obj["age"].as_i64().unwrap();
        assert!(age >= 18 && age <= 60);
    }

    #[test]
    fn resolve_gen_variables_skips_plain_values() {
        use std::collections::BTreeMap;
        let mut vars = BTreeMap::new();
        vars.insert("plain".to_string(), Value::String("hello".to_string()));
        vars.insert(
            "id".to_string(),
            serde_json::json!({ "gen": { "type": "uuid" } }),
        );
        let resolved = resolve_gen_variables(&vars).unwrap();
        assert_eq!(resolved["plain"], Value::String("hello".to_string()));
        assert_eq!(resolved["id"].as_str().unwrap().len(), 36);
    }
}
