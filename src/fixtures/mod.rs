use crate::generator::resolve_gen_values;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureConfig {
    #[serde(rename = "schemaRef", default)]
    pub schema_ref: Option<String>,
    pub build: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FixtureFile {
    fixture: FixtureConfig,
}

/// Load and parse a fixture YAML file.
pub fn load_fixture(path: &Path) -> Result<FixtureConfig, String> {
    if !path.is_file() {
        return Err(format!(
            "fixture_resolution_error: fixture file not found: {}",
            path.display()
        ));
    }
    let content = fs::read_to_string(path).map_err(|e| {
        format!(
            "fixture_resolution_error: failed to read fixture {}: {}",
            path.display(),
            e
        )
    })?;
    let parsed = serde_yaml::from_str::<FixtureFile>(&content).map_err(|e| {
        format!(
            "fixture_resolution_error: invalid fixture structure in {}: {}",
            path.display(),
            e
        )
    })?;
    if !parsed.fixture.build.is_object() {
        return Err(format!(
            "fixture_resolution_error: fixture.build must be an object in {}",
            path.display()
        ));
    }
    Ok(parsed.fixture)
}

/// Materialize a fixture: run gen blocks in build, then apply overrides.
/// Returns the final JSON body.
pub fn materialize_fixture(
    config: &FixtureConfig,
    overrides: Option<&Value>,
) -> Result<Value, String> {
    let (resolved, errors) = resolve_gen_values(&config.build, "build");
    if !errors.is_empty() {
        return Err(format!(
            "fixture_resolution_error: gen error in fixture build: {}",
            errors.join("; ")
        ));
    }

    let mut result = match resolved {
        Value::Object(map) => map,
        _ => {
            return Err(
                "fixture_resolution_error: fixture.build did not resolve to an object".to_string(),
            )
        }
    };

    if let Some(Value::Object(overrides_map)) = overrides {
        for (k, v) in overrides_map {
            result.insert(k.clone(), v.clone());
        }
    }

    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let p = std::env::temp_dir().join(format!("speq-fixtures-{}-{}", tag, suffix));
        fs::create_dir_all(&p).expect("create tmp dir");
        p
    }

    #[test]
    fn load_fixture_missing_file_errors() {
        let path = std::path::Path::new("/nonexistent/fixture.yaml");
        let result = load_fixture(path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fixture_resolution_error"));
    }

    #[test]
    fn load_fixture_invalid_structure_errors() {
        let dir = tmp_dir("invalid");
        let file = dir.join("bad.yaml");
        fs::write(&file, "not_fixture: true\n").expect("write");
        let result = load_fixture(&file);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fixture_resolution_error"));
    }

    #[test]
    fn load_fixture_build_not_object_errors() {
        let dir = tmp_dir("not-obj");
        let file = dir.join("bad.yaml");
        fs::write(&file, "fixture:\n  build: \"a string\"\n").expect("write");
        let result = load_fixture(&file);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fixture_resolution_error"));
    }

    #[test]
    fn load_fixture_static_build_succeeds() {
        let dir = tmp_dir("static");
        let file = dir.join("f.yaml");
        fs::write(
            &file,
            "fixture:\n  build:\n    name: Alice\n    age: 30\n",
        )
        .expect("write");
        let cfg = load_fixture(&file).expect("should load");
        assert_eq!(cfg.build["name"], Value::String("Alice".to_string()));
        assert_eq!(cfg.build["age"], serde_json::json!(30));
    }

    #[test]
    fn load_fixture_with_schema_ref() {
        let dir = tmp_dir("schema-ref");
        let file = dir.join("f.yaml");
        fs::write(
            &file,
            "fixture:\n  schemaRef: \"user.yaml\"\n  build:\n    id: 1\n",
        )
        .expect("write");
        let cfg = load_fixture(&file).expect("should load");
        assert_eq!(cfg.schema_ref.as_deref(), Some("user.yaml"));
    }

    #[test]
    fn materialize_fixture_static_fields() {
        let cfg = FixtureConfig {
            schema_ref: None,
            build: serde_json::json!({ "name": "Bob", "active": true }),
        };
        let result = materialize_fixture(&cfg, None).expect("materialize");
        assert_eq!(result["name"], Value::String("Bob".to_string()));
        assert_eq!(result["active"], Value::Bool(true));
    }

    #[test]
    fn materialize_fixture_overrides_applied() {
        let cfg = FixtureConfig {
            schema_ref: None,
            build: serde_json::json!({ "email": "original@example.com", "name": "Alice" }),
        };
        let overrides = serde_json::json!({ "email": "custom@example.com" });
        let result = materialize_fixture(&cfg, Some(&overrides)).expect("materialize");
        assert_eq!(
            result["email"],
            Value::String("custom@example.com".to_string())
        );
        assert_eq!(result["name"], Value::String("Alice".to_string()));
    }

    #[test]
    fn materialize_fixture_gen_blocks_resolved() {
        let cfg = FixtureConfig {
            schema_ref: None,
            build: serde_json::json!({
                "id": { "gen": { "type": "uuid" } },
                "email": { "gen": { "type": "email" } }
            }),
        };
        let result = materialize_fixture(&cfg, None).expect("materialize");
        let id = result["id"].as_str().expect("id is string");
        assert_eq!(id.len(), 36, "expected uuid");
        let email = result["email"].as_str().expect("email is string");
        assert!(email.contains('@'), "expected email format");
    }

    #[test]
    fn materialize_fixture_overrides_win_over_gen() {
        let cfg = FixtureConfig {
            schema_ref: None,
            build: serde_json::json!({
                "email": { "gen": { "type": "email" } }
            }),
        };
        let overrides = serde_json::json!({ "email": "fixed@test.com" });
        let result = materialize_fixture(&cfg, Some(&overrides)).expect("materialize");
        assert_eq!(
            result["email"],
            Value::String("fixed@test.com".to_string())
        );
    }

    #[test]
    fn full_fixture_load_and_materialize_with_override() {
        let dir = tmp_dir("full");
        let file = dir.join("create-user.yaml");
        fs::write(
            &file,
            concat!(
                "fixture:\n",
                "  build:\n",
                "    email:\n",
                "      gen:\n",
                "        type: email\n",
                "    name: DefaultName\n",
                "    isActive: true\n"
            ),
        )
        .expect("write fixture");
        let cfg = load_fixture(&file).expect("load fixture");
        let overrides = serde_json::json!({ "email": "override@test.com" });
        let body = materialize_fixture(&cfg, Some(&overrides)).expect("materialize");
        assert_eq!(body["email"], Value::String("override@test.com".to_string()));
        assert_eq!(body["name"], Value::String("DefaultName".to_string()));
        assert_eq!(body["isActive"], Value::Bool(true));
    }
}
