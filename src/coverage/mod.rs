use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// A single OpenAPI endpoint: (method, path_template).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
}

/// Coverage config parsed from manifest.yaml `coverage:` block.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoverageConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Path to OpenAPI spec file (relative to speq root).
    #[serde(default)]
    pub openapi: Option<String>,
    #[serde(default)]
    pub report: bool,
    /// Optional minimum coverage percentage; exit non-zero if below.
    #[serde(default)]
    pub fail_below: Option<f64>,
}

/// Computed coverage result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub enabled: bool,
    pub total_endpoints: usize,
    pub covered_endpoints: usize,
    pub percentage: f64,
    pub uncovered: Vec<Endpoint>,
}

/// Parse an OpenAPI/Swagger document (YAML or JSON) and extract all (method, path) pairs.
/// Skips entries with `x-internal: true` at the path-item or operation level.
pub fn parse_openapi_endpoints(content: &str, source_hint: &str) -> Result<Vec<Endpoint>, String> {
    let doc: Value = if source_hint.ends_with(".json") {
        serde_json::from_str(content).map_err(|e| format!("JSON parse error in {}: {e}", source_hint))?
    } else {
        serde_yaml::from_str(content).map_err(|e| format!("YAML parse error in {}: {e}", source_hint))?
    };

    let paths = match doc.get("paths") {
        Some(Value::Object(p)) => p,
        _ => return Err(format!("no 'paths' object found in OpenAPI spec: {}", source_hint)),
    };

    let http_methods = ["get", "post", "put", "patch", "delete", "head", "options", "trace"];
    let mut endpoints = Vec::new();

    for (path, path_item) in paths {
        // Skip path items with x-internal: true at path level.
        if let Some(Value::Bool(true)) = path_item.get("x-internal") {
            continue;
        }
        if let Some(Value::Object(item_map)) = Some(path_item) {
            for method in &http_methods {
                if let Some(operation) = item_map.get(*method) {
                    // Skip operations with x-internal: true.
                    if let Some(Value::Bool(true)) = operation.get("x-internal") {
                        continue;
                    }
                    endpoints.push(Endpoint {
                        method: method.to_ascii_uppercase(),
                        path: path.clone(),
                    });
                }
            }
        }
    }

    Ok(endpoints)
}

/// Compile an OpenAPI path template like `/users/{id}` into a Regex matching that pattern.
fn compile_path_template(template: &str) -> Regex {
    let escaped = regex::escape(template);
    // Replace escaped `\{...\}` param placeholders with a segment-matching group.
    let pattern = escaped.replace(r"\{", "{").replace(r"\}", "}");
    let pattern = Regex::new(r"\{[^}]+\}").unwrap().replace_all(&pattern, "[^/]+");
    Regex::new(&format!("^{}$", pattern)).unwrap_or_else(|_| Regex::new("^$").unwrap())
}

/// Strip base URL prefix from a test URL to get just the path.
pub fn strip_base_url<'a>(url: &'a str, base_url: &str) -> &'a str {
    let base = base_url.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix(base) {
        if rest.is_empty() { return "/"; }
        // Keep only path, drop query string.
        let path = rest.split('?').next().unwrap_or(rest);
        return path;
    }
    // If base URL not matched, try stripping scheme+host portion.
    if let Some(path_start) = url.find("://").and_then(|s| url[s + 3..].find('/').map(|p| s + 3 + p)) {
        let path = &url[path_start..];
        let path = path.split('?').next().unwrap_or(path);
        return path;
    }
    url
}

/// Determine which OpenAPI endpoints were covered by the executed requests.
pub fn compute_coverage(
    openapi_endpoints: &[Endpoint],
    executed: &HashSet<(String, String)>,
    base_url: &str,
) -> CoverageReport {
    let templates: Vec<(Endpoint, Regex)> = openapi_endpoints
        .iter()
        .map(|e| (e.clone(), compile_path_template(&e.path)))
        .collect();

    let mut covered: HashSet<usize> = HashSet::new();
    for (method, url) in executed {
        let path = strip_base_url(url, base_url);
        let method_upper = method.to_ascii_uppercase();
        for (idx, (endpoint, pattern)) in templates.iter().enumerate() {
            if endpoint.method == method_upper && pattern.is_match(path) {
                covered.insert(idx);
            }
        }
    }

    let total = openapi_endpoints.len();
    let covered_count = covered.len();
    let percentage = if total == 0 { 100.0 } else { covered_count as f64 / total as f64 * 100.0 };
    let uncovered = openapi_endpoints
        .iter()
        .enumerate()
        .filter(|(idx, _)| !covered.contains(idx))
        .map(|(_, e)| e.clone())
        .collect();

    CoverageReport {
        enabled: true,
        total_endpoints: total,
        covered_endpoints: covered_count,
        percentage,
        uncovered,
    }
}

/// Load and parse an OpenAPI spec from a file path relative to speq root.
pub fn load_openapi_endpoints(speq_root: &Path, openapi_path: &str) -> Result<Vec<Endpoint>, String> {
    let full_path = if std::path::Path::new(openapi_path).is_absolute() {
        std::path::PathBuf::from(openapi_path)
    } else {
        speq_root.join(openapi_path)
    };
    let content = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("failed to read OpenAPI file {}: {e}", full_path.display()))?;
    let hint = full_path.to_string_lossy().to_string();
    parse_openapi_endpoints(&content, &hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_openapi_yaml() {
        let yaml = r#"
openapi: "3.0.0"
paths:
  /users:
    get:
      summary: list users
    post:
      summary: create user
  /users/{id}:
    get:
      summary: get user
    delete:
      summary: delete user
"#;
        let endpoints = parse_openapi_endpoints(yaml, "spec.yaml").unwrap();
        assert_eq!(endpoints.len(), 4);
        assert!(endpoints.iter().any(|e| e.method == "GET" && e.path == "/users"));
        assert!(endpoints.iter().any(|e| e.method == "DELETE" && e.path == "/users/{id}"));
    }

    #[test]
    fn parse_openapi_skips_x_internal() {
        let yaml = r#"
openapi: "3.0.0"
paths:
  /public:
    get:
      summary: public
  /internal:
    x-internal: true
    get:
      summary: internal
  /mixed:
    get:
      x-internal: true
      summary: internal op
    post:
      summary: public post
"#;
        let endpoints = parse_openapi_endpoints(yaml, "spec.yaml").unwrap();
        assert!(endpoints.iter().any(|e| e.path == "/public"));
        assert!(!endpoints.iter().any(|e| e.path == "/internal"));
        // /mixed GET is internal, POST is not
        assert!(!endpoints.iter().any(|e| e.path == "/mixed" && e.method == "GET"));
        assert!(endpoints.iter().any(|e| e.path == "/mixed" && e.method == "POST"));
    }

    #[test]
    fn strip_base_url_removes_prefix() {
        assert_eq!(strip_base_url("https://api.example.com/users/1", "https://api.example.com"), "/users/1");
        assert_eq!(strip_base_url("https://api.example.com/users?page=1", "https://api.example.com"), "/users");
    }

    #[test]
    fn coverage_matches_path_templates() {
        let endpoints = vec![
            Endpoint { method: "GET".into(), path: "/users".into() },
            Endpoint { method: "DELETE".into(), path: "/users/{id}".into() },
            Endpoint { method: "POST".into(), path: "/users".into() },
        ];
        let mut executed = HashSet::new();
        executed.insert(("GET".into(), "https://api.example.com/users".into()));
        executed.insert(("DELETE".into(), "https://api.example.com/users/42".into()));

        let report = compute_coverage(&endpoints, &executed, "https://api.example.com");
        assert_eq!(report.total_endpoints, 3);
        assert_eq!(report.covered_endpoints, 2);
        assert!((report.percentage - 66.666_666).abs() < 0.01);
        assert_eq!(report.uncovered.len(), 1);
        assert_eq!(report.uncovered[0].method, "POST");
    }
}
