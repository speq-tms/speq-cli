use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn parse_mode(mode: Option<String>) -> Result<String, String> {
    match mode.as_deref() {
        None => Ok("in-repo".to_string()),
        Some("in-repo") => Ok("in-repo".to_string()),
        Some("test-repo") => Ok("test-repo".to_string()),
        Some(other) => Err(format!(
            "unsupported mode '{}', expected 'in-repo' or 'test-repo'",
            other
        )),
    }
}

fn target_root(cwd: &Path, mode: &str) -> PathBuf {
    if mode == "test-repo" {
        cwd.to_path_buf()
    } else {
        cwd.join(".speq")
    }
}

fn project_name_from_cwd(cwd: &Path) -> String {
    cwd.file_name()
        .and_then(|x| x.to_str())
        .map(|x| x.to_string())
        .unwrap_or_else(|| "speq-project".to_string())
}

fn write_manifest(root: &Path, project: &str) -> Result<(), String> {
    let manifest_path = root.join("manifest.yaml");
    if manifest_path.exists() {
        return Err(format!(
            "manifest already exists: {}",
            manifest_path.display()
        ));
    }

    let manifest = serde_json::json!({
        "version": "1",
        "project": project,
        "defaultEnvironment": "ci",
        "environmentsDir": "environments",
        "suitesDir": "suites",
        "reportsDir": "reports"
    });
    let yaml = serde_yaml::to_string(
        &serde_json::from_value::<Value>(manifest)
            .map_err(|e| format!("internal: failed to prepare manifest: {e}"))?,
    )
    .map_err(|e| format!("internal: failed to encode manifest yaml: {e}"))?;
    fs::write(&manifest_path, yaml)
        .map_err(|e| format!("failed to write manifest {}: {e}", manifest_path.display()))
}

fn write_default_env(root: &Path) -> Result<(), String> {
    let env_path = root.join("environments").join("ci.yaml");
    if env_path.exists() {
        return Ok(());
    }
    let body = "name: ci\nbaseUrl: https://httpbin.org\n";
    fs::write(&env_path, body).map_err(|e| format!("failed to write {}: {e}", env_path.display()))
}

fn write_sample_suite(root: &Path) -> Result<(), String> {
    let suite_path = root.join("suites").join("smoke.yaml");
    if suite_path.exists() {
        return Ok(());
    }
    let body = r#"id: "smoke.health"
title: "Health endpoint smoke test"
steps:
  - type: api
    name: "GET health"
    method: GET
    url: "/status/200"
    assert:
      - type: status
        expected: 200
"#;
    fs::write(&suite_path, body).map_err(|e| format!("failed to write {}: {e}", suite_path.display()))
}

pub fn command_init(mode: Option<String>) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("internal: failed to read cwd: {e}"))?;
    let selected_mode = parse_mode(mode)?;
    let root = target_root(&cwd, &selected_mode);

    fs::create_dir_all(root.join("environments"))
        .map_err(|e| format!("failed to create environments dir: {e}"))?;
    fs::create_dir_all(root.join("suites")).map_err(|e| format!("failed to create suites dir: {e}"))?;
    fs::create_dir_all(root.join("reports")).map_err(|e| format!("failed to create reports dir: {e}"))?;

    let project = project_name_from_cwd(&cwd);
    write_manifest(&root, &project)?;
    write_default_env(&root)?;
    write_sample_suite(&root)?;

    println!("Initialized speq ({}) at {}", selected_mode, root.display());
    Ok(())
}
