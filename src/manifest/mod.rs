use crate::coverage::CoverageConfig;
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

/// Default total request budget when nothing configures one. reqwest itself has
/// no default, so without this a hung service hangs the whole run.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Default connection-establishment budget, a subset of the total budget.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;

/// `followRedirects` accepts either a bool (`false` = never, `true` = the
/// default hop limit) or an integer maximum number of hops.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FollowRedirects {
    Enabled(bool),
    MaxHops(u32),
}

impl FollowRedirects {
    /// Hop limit this setting implies; `0` means redirects are not followed.
    pub fn max_hops(&self) -> u32 {
        match self {
            FollowRedirects::Enabled(true) => DEFAULT_REDIRECT_HOPS,
            FollowRedirects::Enabled(false) => 0,
            FollowRedirects::MaxHops(n) => *n,
        }
    }
}

pub const DEFAULT_REDIRECT_HOPS: u32 = 10;

/// Transport policy for outbound requests. Declared in `manifest.yaml` and
/// narrowed per environment; `timeoutMs` is narrowed once more per step.
///
/// Every field is optional so that the two levels can be merged field by field
/// without a partially-specified environment block erasing the manifest's.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpConfig {
    pub timeout_ms: Option<u64>,
    pub connect_timeout_ms: Option<u64>,
    pub follow_redirects: Option<FollowRedirects>,
    pub proxy: Option<String>,
    /// PEM bundle of additional roots to trust, relative to the speq root.
    pub ca_file: Option<String>,
    /// Client certificate and its key, both PEM, for mutual TLS.
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    /// Disables certificate verification. Never use outside a throwaway
    /// environment — it is accepted so that self-signed staging APIs are
    /// testable, not because it is safe.
    pub insecure_skip_verify: Option<bool>,
}

impl HttpConfig {
    /// Field-wise override: anything `narrower` states wins, anything it leaves
    /// unset falls through to `self`.
    pub fn merged_with(&self, narrower: Option<&HttpConfig>) -> HttpConfig {
        let Some(n) = narrower else { return self.clone() };
        HttpConfig {
            timeout_ms: n.timeout_ms.or(self.timeout_ms),
            connect_timeout_ms: n.connect_timeout_ms.or(self.connect_timeout_ms),
            follow_redirects: n.follow_redirects.clone().or_else(|| self.follow_redirects.clone()),
            proxy: n.proxy.clone().or_else(|| self.proxy.clone()),
            ca_file: n.ca_file.clone().or_else(|| self.ca_file.clone()),
            client_cert: n.client_cert.clone().or_else(|| self.client_cert.clone()),
            client_key: n.client_key.clone().or_else(|| self.client_key.clone()),
            insecure_skip_verify: n.insecure_skip_verify.or(self.insecure_skip_verify),
        }
    }

    pub fn timeout_ms_or_default(&self) -> u64 {
        self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)
    }

    pub fn connect_timeout_ms_or_default(&self) -> u64 {
        self.connect_timeout_ms.unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS)
    }

    /// Rejects blocks that parse but cannot produce a working client, so that
    /// `speq validate` catches them instead of the first request of a run.
    pub fn validate(&self, location: &str) -> Result<(), String> {
        if let Some(0) = self.timeout_ms {
            return Err(format!("http.timeoutMs must be > 0 in {location}"));
        }
        if let Some(0) = self.connect_timeout_ms {
            return Err(format!("http.connectTimeoutMs must be > 0 in {location}"));
        }
        if let (Some(total), Some(connect)) = (self.timeout_ms, self.connect_timeout_ms) {
            if connect > total {
                return Err(format!(
                    "http.connectTimeoutMs ({connect}) must be <= http.timeoutMs ({total}) in {location}"
                ));
            }
        }
        if let Some(proxy) = &self.proxy {
            if proxy.trim().is_empty() {
                return Err(format!("http.proxy must not be empty in {location}"));
            }
            if reqwest::Proxy::all(proxy.trim()).is_err() {
                return Err(format!(
                    "http.proxy '{proxy}' is not a usable proxy URL in {location}"
                ));
            }
        }
        match (&self.client_cert, &self.client_key) {
            (Some(_), None) => {
                return Err(format!(
                    "http.clientCert requires http.clientKey in {location}"
                ))
            }
            (None, Some(_)) => {
                return Err(format!(
                    "http.clientKey requires http.clientCert in {location}"
                ))
            }
            _ => {}
        }
        Ok(())
    }
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
    #[serde(default)]
    pub http: Option<HttpConfig>,
    #[serde(default)]
    pub coverage: Option<CoverageConfig>,
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
    if let Some(http) = &parsed.http {
        http.validate(&manifest_path.display().to_string())?;
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

/// Worst-case wall time one step can consume: every attempt burns the full
/// request budget, plus the backoff waited between them.
///
/// Retry multiplies the timeout, so a run's real patience is not `timeoutMs` —
/// `speq validate` reports this number so the multiplication is never silent.
pub fn worst_case_step_budget_ms(http: Option<&HttpConfig>, retry: Option<&RetryConfig>) -> u64 {
    let timeout = http.map(|h| h.timeout_ms_or_default()).unwrap_or(DEFAULT_TIMEOUT_MS);
    let Some(retry) = retry.filter(|r| r.enabled && r.max_attempts > 0) else {
        return timeout;
    };
    let attempts = retry.max_attempts.max(1) as u64;
    let mut total = timeout.saturating_mul(attempts);
    for attempt in 1..retry.max_attempts.max(1) {
        let delay = match retry.backoff {
            BackoffStrategy::Fixed => retry.delay_ms,
            BackoffStrategy::Exponential => {
                retry.delay_ms.saturating_mul(1u64 << (attempt - 1).min(16))
            }
        };
        total = total.saturating_add(delay);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrower_http_settings_override_field_by_field() {
        let manifest = HttpConfig {
            timeout_ms: Some(30_000),
            connect_timeout_ms: Some(10_000),
            proxy: Some("http://proxy.internal:8080".to_string()),
            ..Default::default()
        };
        let env = HttpConfig { timeout_ms: Some(2_000), ..Default::default() };
        let merged = manifest.merged_with(Some(&env));

        assert_eq!(merged.timeout_ms, Some(2_000), "environment narrows the timeout");
        assert_eq!(
            merged.connect_timeout_ms,
            Some(10_000),
            "a field the environment leaves unset falls through to the manifest"
        );
        assert_eq!(merged.proxy.as_deref(), Some("http://proxy.internal:8080"));
    }

    #[test]
    fn follow_redirects_accepts_a_bool_or_a_hop_count() {
        assert_eq!(FollowRedirects::Enabled(false).max_hops(), 0);
        assert_eq!(FollowRedirects::Enabled(true).max_hops(), DEFAULT_REDIRECT_HOPS);
        assert_eq!(FollowRedirects::MaxHops(3).max_hops(), 3);

        let parsed: HttpConfig = serde_yaml::from_str("followRedirects: 4").expect("hop count parses");
        assert_eq!(parsed.follow_redirects, Some(FollowRedirects::MaxHops(4)));
        let parsed: HttpConfig = serde_yaml::from_str("followRedirects: false").expect("bool parses");
        assert_eq!(parsed.follow_redirects, Some(FollowRedirects::Enabled(false)));
    }

    #[test]
    fn malformed_http_blocks_are_rejected() {
        let zero = HttpConfig { timeout_ms: Some(0), ..Default::default() };
        assert!(zero.validate("m.yaml").unwrap_err().contains("timeoutMs must be > 0"));

        let inverted = HttpConfig {
            timeout_ms: Some(1_000),
            connect_timeout_ms: Some(5_000),
            ..Default::default()
        };
        assert!(inverted.validate("m.yaml").unwrap_err().contains("must be <="));

        let half_identity = HttpConfig {
            client_cert: Some("client.pem".to_string()),
            ..Default::default()
        };
        assert!(half_identity.validate("m.yaml").unwrap_err().contains("requires http.clientKey"));

        let unknown = serde_yaml::from_str::<HttpConfig>("timeoutMS: 500");
        assert!(unknown.is_err(), "a misspelled key must not be silently ignored");
    }

    #[test]
    fn worst_case_budget_accounts_for_retry_attempts_and_backoff() {
        let http = HttpConfig { timeout_ms: Some(1_000), ..Default::default() };
        let retry = RetryConfig {
            enabled: true,
            max_attempts: 3,
            delay_ms: 100,
            backoff: BackoffStrategy::Exponential,
            retry_on: RetryOn::default(),
        };
        // 3 x 1000ms of request budget + 100ms + 200ms of backoff.
        assert_eq!(worst_case_step_budget_ms(Some(&http), Some(&retry)), 3_300);
        assert_eq!(worst_case_step_budget_ms(Some(&http), None), 1_000);
        assert_eq!(worst_case_step_budget_ms(None, None), DEFAULT_TIMEOUT_MS);
    }
}
