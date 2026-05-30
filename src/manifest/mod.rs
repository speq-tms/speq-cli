use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RetryOn {
    #[serde(default)]
    pub network_errors: bool,
    #[serde(default)]
    pub status_codes: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackoffStrategy {
    #[default]
    Fixed,
    Exponential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "maxAttempts", default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(rename = "delayMs", default)]
    pub delay_ms: u64,
    #[serde(default)]
    pub backoff: BackoffStrategy,
    #[serde(rename = "retryOn", default)]
    pub retry_on: RetryOn,
}

fn default_max_attempts() -> u32 {
    3
}

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
    #[serde(rename = "schemasDir", default)]
    pub schemas_dir: Option<String>,
    #[serde(rename = "modulesDir", default)]
    pub modules_dir: Option<String>,
    #[serde(rename = "fixturesDir", default)]
    pub fixtures_dir: Option<String>,
    #[serde(default)]
    pub retry: Option<RetryConfig>,
}

impl Manifest {
    pub fn environments_dir_or_default(&self) -> String {
        self.environments_dir
            .clone()
            .unwrap_or_else(|| "environments".to_string())
    }

    pub fn suites_dir_or_default(&self) -> String {
        self.suites_dir
            .clone()
            .unwrap_or_else(|| "suites".to_string())
    }

    pub fn reports_dir_or_default(&self) -> String {
        self.reports_dir
            .clone()
            .unwrap_or_else(|| "reports".to_string())
    }

    pub fn schemas_dir_or_default(&self) -> String {
        self.schemas_dir
            .clone()
            .unwrap_or_else(|| "schemas".to_string())
    }

    pub fn modules_dir_or_default(&self) -> String {
        self.modules_dir
            .clone()
            .unwrap_or_else(|| "modules".to_string())
    }

    pub fn fixtures_dir_or_default(&self) -> String {
        self.fixtures_dir
            .clone()
            .unwrap_or_else(|| "fixtures".to_string())
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
    if let Some(retry) = &parsed.retry {
        if retry.max_attempts == 0 {
            return Err(format!(
                "retry.maxAttempts must be > 0 in {}",
                manifest_path.display()
            ));
        }
    }
    Ok(parsed)
}
