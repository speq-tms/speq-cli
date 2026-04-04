use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub project: String,
    #[serde(rename = "defaultEnvironment")]
    pub default_environment: String,
    #[serde(rename = "environmentsDir", default)]
    pub environments_dir: Option<String>,
    #[serde(rename = "suitesDir", default)]
    pub suites_dir: Option<String>,
    #[serde(rename = "reportsDir", default)]
    pub reports_dir: Option<String>,
}

impl Manifest {
    pub fn suites_dir_or_default(&self) -> String {
        self.suites_dir
            .clone()
            .unwrap_or_else(|| "suites".to_string())
    }
}

pub fn read_manifest(speq_root: &Path) -> Result<Manifest, String> {
    let manifest_path = speq_root.join("manifest.yaml");
    let content = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read manifest {}: {e}", manifest_path.display()))?;
    let parsed = serde_yaml::from_str::<Manifest>(&content)
        .map_err(|e| format!("invalid manifest {}: {e}", manifest_path.display()))?;

    if parsed.version.trim() != "1" {
        return Err(format!(
            "unsupported manifest version '{}' in {} (expected '1')",
            parsed.version,
            manifest_path.display()
        ));
    }
    if parsed.project.trim().is_empty() {
        return Err(format!("manifest field 'project' is required: {}", manifest_path.display()));
    }
    if parsed.default_environment.trim().is_empty() {
        return Err(format!(
            "manifest field 'defaultEnvironment' is required: {}",
            manifest_path.display()
        ));
    }
    Ok(parsed)
}
