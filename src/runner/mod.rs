use crate::parser::{parse_reusable_steps, Assertion, ImportSpec, Step, TestSpec};
use jsonschema::JSONSchema;
use regex::Regex;
use reqwest::Method;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpRequestInfo {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpResponseInfo {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssertionRunResult {
    #[serde(rename = "type")]
    pub assertion_type: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRunResult {
    pub name: String,
    pub status: String,
    pub message: String,
    pub response_status: Option<u16>,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<HttpRequestInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<HttpResponseInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assertions: Vec<AssertionRunResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestRunResult {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub file: String,
    pub status: String,
    pub duration_ms: u128,
    pub errors: Vec<String>,
    pub steps: Vec<StepRunResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup_steps: Vec<StepRunResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teardown_steps: Vec<StepRunResult>,
}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub schemas_root: PathBuf,
    pub modules_root: PathBuf,
}

#[derive(Debug, Default)]
struct RuntimeCaches {
    schema_cache: HashMap<String, JSONSchema>,
    module_cache: HashMap<String, ModuleSpec>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModuleSpec {
    #[serde(default)]
    variables: BTreeMap<String, Value>,
    #[serde(default)]
    actions: BTreeMap<String, ModuleActionSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ModuleActionSpec {
    Legacy(Vec<Step>),
    Detailed(ModuleActionDetailed),
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModuleActionDetailed {
    #[serde(default)]
    properties: Vec<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Debug, Clone)]
struct ResolvedAction {
    steps: Vec<Step>,
    required_properties: Vec<String>,
}

fn render_template(input: &str, vars: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    let mut start = 0usize;
    while let Some(open_rel) = input[start..].find("{{") {
        let open = start + open_rel;
        out.push_str(&input[start..open]);
        let rest = &input[open + 2..];
        if let Some(close_rel) = rest.find("}}") {
            let close = open + 2 + close_rel;
            let key = input[open + 2..close].trim();
            if let Some(value) = vars.get(key) {
                let rendered = match value {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push_str(&rendered);
            } else {
                out.push_str(&input[open..close + 2]);
            }
            start = close + 2;
            continue;
        }
        out.push_str(&input[open..]);
        return out;
    }
    out.push_str(&input[start..]);
    out
}

fn json_path_get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path == "$" {
        return Some(root);
    }
    let trimmed = path.strip_prefix("$.")?;
    let mut current = root;
    for part in trimmed.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn module_alias(import: &ImportSpec) -> String {
    if let Some(alias) = &import.alias {
        let trimmed = alias.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    import
        .module
        .rsplit('/')
        .next()
        .map(|x| x.to_string())
        .unwrap_or_else(|| import.module.clone())
}

fn module_path_candidates(modules_root: &Path, module_name: &str) -> Vec<PathBuf> {
    let module_rel = PathBuf::from(module_name);
    let has_ext = module_rel.extension().and_then(|x| x.to_str()).is_some();
    let mut candidates = Vec::new();
    candidates.push(modules_root.join(&module_rel));
    if !has_ext {
        candidates.push(modules_root.join(format!("{module_name}.yaml")));
        candidates.push(modules_root.join(format!("{module_name}.yml")));
    }
    candidates
}

fn schema_path_candidates(schemas_root: &Path, schema_ref: &str) -> Vec<PathBuf> {
    let schema_rel = PathBuf::from(schema_ref);
    let has_ext = schema_rel.extension().and_then(|x| x.to_str()).is_some();
    let mut candidates = Vec::new();
    candidates.push(schemas_root.join(&schema_rel));
    if !has_ext {
        candidates.push(schemas_root.join(format!("{schema_ref}.json")));
        candidates.push(schemas_root.join(format!("{schema_ref}.yaml")));
        candidates.push(schemas_root.join(format!("{schema_ref}.yml")));
    }
    candidates
}

fn read_first_existing_file(candidates: &[PathBuf], label: &str) -> Result<(PathBuf, String), String> {
    for path in candidates {
        if path.is_file() {
            let content =
                fs::read_to_string(path).map_err(|e| format!("failed to read {label} {}: {e}", path.display()))?;
            return Ok((path.clone(), content));
        }
    }
    let options = candidates
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!("{label} file not found, tried: {options}"))
}

fn load_module_spec(
    module_name: &str,
    runtime_paths: &RuntimePaths,
    module_cache: &mut HashMap<String, ModuleSpec>,
) -> Result<ModuleSpec, String> {
    if let Some(cached) = module_cache.get(module_name) {
        return Ok(cached.clone());
    }
    let candidates = module_path_candidates(&runtime_paths.modules_root, module_name);
    let (path, content) = read_first_existing_file(&candidates, "module")?;
    let parsed = serde_yaml::from_str::<ModuleSpec>(&content)
        .map_err(|e| format!("invalid module yaml {}: {e}", path.display()))?;
    module_cache.insert(module_name.to_string(), parsed.clone());
    Ok(parsed)
}

fn collect_import_variables(
    imports: &[ImportSpec],
    runtime_paths: &RuntimePaths,
    module_cache: &mut HashMap<String, ModuleSpec>,
) -> Result<BTreeMap<String, Value>, String> {
    let mut vars = BTreeMap::new();
    for import in imports {
        let module = load_module_spec(&import.module, runtime_paths, module_cache)?;
        for (key, value) in module.variables {
            vars.insert(key, value);
        }
    }
    Ok(vars)
}

fn resolve_action_steps(
    step: &Step,
    imports: &[ImportSpec],
    runtime_paths: &RuntimePaths,
    module_cache: &mut HashMap<String, ModuleSpec>,
) -> Result<ResolvedAction, String> {
    let action = step.action.as_deref().map(str::trim).unwrap_or_default();
    if action.is_empty() {
        return Err("use action is empty".to_string());
    }
    let (alias, action_name) = action.split_once('.').ok_or_else(|| {
        format!(
            "use action '{}' must be in '<alias>.<action>' format for step '{}'",
            action, step.name
        )
    })?;
    let import = imports
        .iter()
        .rev()
        .find(|item| module_alias(item) == alias)
        .ok_or_else(|| {
            let aliases = imports.iter().map(module_alias).collect::<Vec<_>>().join(", ");
            format!(
                "use action '{}' references unknown alias '{}'; available imports: [{}]",
                action, alias, aliases
            )
        })?;
    let module = load_module_spec(&import.module, runtime_paths, module_cache)?;
    let action_spec = module.actions.get(action_name).cloned().ok_or_else(|| {
        format!(
            "action '{}' is not exported by module '{}' for step '{}'",
            action_name, import.module, step.name
        )
    })?;
    let resolved = match action_spec {
        ModuleActionSpec::Legacy(steps) => ResolvedAction {
            steps,
            required_properties: Vec::new(),
        },
        ModuleActionSpec::Detailed(action) => ResolvedAction {
            steps: action.steps,
            required_properties: action.properties,
        },
    };
    Ok(resolved)
}

fn resolve_ref_schema(
    schema_ref: &str,
    runtime_paths: &RuntimePaths,
    schema_cache: &mut HashMap<String, JSONSchema>,
) -> Result<String, String> {
    let candidates = schema_path_candidates(&runtime_paths.schemas_root, schema_ref);
    let (path, content) = read_first_existing_file(&candidates, "schema")?;
    let cache_key = path.to_string_lossy().to_string();
    if !schema_cache.contains_key(&cache_key) {
        let schema_json = serde_yaml::from_str::<Value>(&content)
            .map_err(|e| format!("invalid schema file {}: {e}", path.display()))?;
        let compiled = JSONSchema::compile(&schema_json)
            .map_err(|e| format!("invalid JSON Schema {}: {e}", path.display()))?;
        schema_cache.insert(cache_key.clone(), compiled);
    }
    if schema_cache.contains_key(&cache_key) {
        Ok(cache_key)
    } else {
        Err(format!("internal: failed to cache schema {}", path.display()))
    }
}

fn build_action_vars(
    base_vars: &BTreeMap<String, Value>,
    step: &Step,
    required_properties: &[String],
) -> Result<BTreeMap<String, Value>, String> {
    let mut action_vars = base_vars.clone();
    for (key, value) in &step.properties {
        let merged_value = if let Some(raw) = value.as_str() {
            Value::String(render_template(raw, base_vars))
        } else {
            value.clone()
        };
        action_vars.insert(key.clone(), merged_value);
    }
    for required in required_properties {
        if !action_vars.contains_key(required) {
            return Err(format!(
                "use action '{}' requires property '{}' in step '{}'",
                step.action.as_deref().unwrap_or_default(),
                required,
                step.name
            ));
        }
    }
    Ok(action_vars)
}

fn run_assertions(
    assertions: &[Assertion],
    status: u16,
    body: &str,
    runtime_paths: &RuntimePaths,
    schema_cache: &mut HashMap<String, JSONSchema>,
) -> (Vec<AssertionRunResult>, Vec<String>) {
    let mut details = Vec::new();
    let mut errors = Vec::new();
    let parsed_json = serde_json::from_str::<Value>(body).ok();

    for assertion in assertions {
        match assertion.assertion_type.as_str() {
            "status" => {
                let expected = assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_u64)
                    .map(|v| v as u16);
                if expected != Some(status) {
                    let message = format!(
                        "status assertion failed: expected {:?}, got {}",
                        expected, status
                    );
                    errors.push(message.clone());
                    details.push(AssertionRunResult {
                        assertion_type: "status".to_string(),
                        status: "failed".to_string(),
                        message,
                        path: None,
                        expected: assertion.expected.clone(),
                    });
                } else {
                    details.push(AssertionRunResult {
                        assertion_type: "status".to_string(),
                        status: "passed".to_string(),
                        message: format!("status assertion passed: {}", status),
                        path: None,
                        expected: assertion.expected.clone(),
                    });
                }
            }
            "contains" => {
                let needle = assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !body.contains(needle) {
                    let message = format!("contains assertion failed: '{}' not found", needle);
                    errors.push(message.clone());
                    details.push(AssertionRunResult {
                        assertion_type: "contains".to_string(),
                        status: "failed".to_string(),
                        message,
                        path: None,
                        expected: assertion.expected.clone(),
                    });
                } else {
                    details.push(AssertionRunResult {
                        assertion_type: "contains".to_string(),
                        status: "passed".to_string(),
                        message: format!("contains assertion passed: '{}'", needle),
                        path: None,
                        expected: assertion.expected.clone(),
                    });
                }
            }
            "notcontains" => {
                let needle = assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if body.contains(needle) {
                    let message = format!("notcontains assertion failed: '{}' found", needle);
                    errors.push(message.clone());
                    details.push(AssertionRunResult {
                        assertion_type: "notcontains".to_string(),
                        status: "failed".to_string(),
                        message,
                        path: None,
                        expected: assertion.expected.clone(),
                    });
                } else {
                    details.push(AssertionRunResult {
                        assertion_type: "notcontains".to_string(),
                        status: "passed".to_string(),
                        message: format!("notcontains assertion passed: '{}'", needle),
                        path: None,
                        expected: assertion.expected.clone(),
                    });
                }
            }
            "regex" => {
                let pattern = assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match Regex::new(pattern) {
                    Ok(re) => {
                        if !re.is_match(body) {
                            let message = format!("regex assertion failed: '{}' not matched", pattern);
                            errors.push(message.clone());
                            details.push(AssertionRunResult {
                                assertion_type: "regex".to_string(),
                                status: "failed".to_string(),
                                message,
                                path: None,
                                expected: assertion.expected.clone(),
                            });
                        } else {
                            details.push(AssertionRunResult {
                                assertion_type: "regex".to_string(),
                                status: "passed".to_string(),
                                message: format!("regex assertion passed: '{}'", pattern),
                                path: None,
                                expected: assertion.expected.clone(),
                            });
                        }
                    }
                    Err(e) => {
                        let message = format!("invalid regex '{}': {}", pattern, e);
                        errors.push(message.clone());
                        details.push(AssertionRunResult {
                            assertion_type: "regex".to_string(),
                            status: "failed".to_string(),
                            message,
                            path: None,
                            expected: assertion.expected.clone(),
                        });
                    }
                }
            }
            "exists" => {
                let path = assertion.path.as_deref().unwrap_or("$");
                match &parsed_json {
                    Some(json) if json_path_get(json, path).is_some() => details.push(AssertionRunResult {
                        assertion_type: "exists".to_string(),
                        status: "passed".to_string(),
                        message: format!("exists assertion passed at '{}'", path),
                        path: Some(path.to_string()),
                        expected: None,
                    }),
                    Some(_) => {
                        let message = format!("exists assertion failed: path '{}' not found", path);
                        errors.push(message.clone());
                        details.push(AssertionRunResult {
                            assertion_type: "exists".to_string(),
                            status: "failed".to_string(),
                            message,
                            path: Some(path.to_string()),
                            expected: None,
                        });
                    }
                    None => {
                        let message = "exists assertion failed: response is not json".to_string();
                        errors.push(message.clone());
                        details.push(AssertionRunResult {
                            assertion_type: "exists".to_string(),
                            status: "failed".to_string(),
                            message,
                            path: Some(path.to_string()),
                            expected: None,
                        });
                    }
                }
            }
            "json" => {
                let path = assertion.path.as_deref().unwrap_or("$");
                let expected = assertion.expected.clone().unwrap_or(Value::Null);
                match &parsed_json {
                    Some(json) => match json_path_get(json, path) {
                        Some(actual) if actual == &expected => details.push(AssertionRunResult {
                            assertion_type: "json".to_string(),
                            status: "passed".to_string(),
                            message: format!("json assertion passed at '{}'", path),
                            path: Some(path.to_string()),
                            expected: Some(expected.clone()),
                        }),
                        Some(actual) => {
                            let message = format!(
                                "json assertion failed at '{}': expected {}, got {}",
                                path, expected, actual
                            );
                            errors.push(message.clone());
                            details.push(AssertionRunResult {
                                assertion_type: "json".to_string(),
                                status: "failed".to_string(),
                                message,
                                path: Some(path.to_string()),
                                expected: Some(expected.clone()),
                            });
                        }
                        None => {
                            let message = format!("json assertion failed: path '{}' not found", path);
                            errors.push(message.clone());
                            details.push(AssertionRunResult {
                                assertion_type: "json".to_string(),
                                status: "failed".to_string(),
                                message,
                                path: Some(path.to_string()),
                                expected: Some(expected.clone()),
                            });
                        }
                    },
                    None => {
                        let message = "json assertion failed: response is not json".to_string();
                        errors.push(message.clone());
                        details.push(AssertionRunResult {
                            assertion_type: "json".to_string(),
                            status: "failed".to_string(),
                            message,
                            path: Some(path.to_string()),
                            expected: Some(expected.clone()),
                        });
                    }
                }
            }
            "schema" => {
                let Some(target_json) = parsed_json.as_ref() else {
                    let message = "schema assertion failed: response is not json".to_string();
                    errors.push(message.clone());
                    details.push(AssertionRunResult {
                        assertion_type: "schema".to_string(),
                        status: "failed".to_string(),
                        message,
                        path: None,
                        expected: None,
                    });
                    continue;
                };

                let validate_result = if let Some(schema_ref) = assertion.r#ref.as_deref() {
                    match resolve_ref_schema(schema_ref, runtime_paths, schema_cache) {
                        Ok(cache_key) => {
                            if let Some(schema) = schema_cache.get(&cache_key) {
                                match schema.validate(target_json) {
                                    Ok(()) => Ok(()),
                                    Err(errs) => {
                                        let reason = errs
                                            .take(3)
                                            .map(|e| e.to_string())
                                            .collect::<Vec<_>>()
                                            .join("; ");
                                        Err(reason)
                                    }
                                }
                            } else {
                                Err(format!("internal: schema cache miss for '{schema_ref}'"))
                            }
                        }
                        Err(e) => Err(e),
                    }
                } else if let Some(inline_schema) = assertion.inline.as_ref() {
                    match JSONSchema::compile(inline_schema) {
                        Ok(compiled) => match compiled.validate(target_json) {
                            Ok(()) => Ok(()),
                            Err(errs) => {
                                let reason = errs
                                    .take(3)
                                    .map(|e| e.to_string())
                                    .collect::<Vec<_>>()
                                    .join("; ");
                                Err(reason)
                            }
                        },
                        Err(e) => Err(format!("invalid inline JSON Schema: {e}")),
                    }
                } else {
                    Err("schema assertion requires 'ref' or 'inline'".to_string())
                };

                match validate_result {
                    Ok(()) => {
                        let source = assertion
                            .r#ref
                            .clone()
                            .unwrap_or_else(|| "inline".to_string());
                        details.push(AssertionRunResult {
                            assertion_type: "schema".to_string(),
                            status: "passed".to_string(),
                            message: format!("schema assertion passed ({source})"),
                            path: None,
                            expected: None,
                        });
                    }
                    Err(reason) => {
                        let message = format!("schema assertion failed: {reason}");
                        errors.push(message.clone());
                        details.push(AssertionRunResult {
                            assertion_type: "schema".to_string(),
                            status: "failed".to_string(),
                            message,
                            path: None,
                            expected: None,
                        });
                    }
                }
            }
            other => {
                let message = format!("unsupported assertion type in runtime: {}", other);
                errors.push(message.clone());
                details.push(AssertionRunResult {
                    assertion_type: other.to_string(),
                    status: "failed".to_string(),
                    message,
                    path: assertion.path.clone(),
                    expected: assertion.expected.clone(),
                });
            }
        }
    }
    (details, errors)
}

async fn execute_api_step(
    client: &reqwest::Client,
    vars: &BTreeMap<String, Value>,
    base_url: &str,
    step: &Step,
    runtime_paths: &RuntimePaths,
    cache: &mut RuntimeCaches,
) -> StepRunResult {
    let step_started = Instant::now();
    let method = match Method::from_bytes(step.method.to_ascii_uppercase().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return StepRunResult {
                name: step.name.clone(),
                status: "failed".to_string(),
                message: format!("invalid method '{}'", step.method),
                response_status: None,
                duration_ms: step_started.elapsed().as_millis(),
                request: None,
                response: None,
                assertions: Vec::new(),
            }
        }
    };

    let rendered_url = render_template(&step.url, vars);
    let full_url = if rendered_url.starts_with("http://") || rendered_url.starts_with("https://") {
        rendered_url
    } else {
        format!("{}{}", base_url.trim_end_matches('/'), rendered_url)
    };

    let full_url_for_report = full_url.clone();
    let mut req = client.request(method.clone(), full_url);
    let mut rendered_headers = BTreeMap::new();
    for (k, v) in &step.headers {
        let rendered = render_template(v, vars);
        rendered_headers.insert(k.clone(), rendered.clone());
        req = req.header(k, rendered);
    }
    let request_body = step.body.clone();
    if let Some(body) = &step.body {
        req = req.json(body);
    }

    let response = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            return StepRunResult {
                name: step.name.clone(),
                status: "failed".to_string(),
                message: format!("request failed: {}", e),
                response_status: None,
                duration_ms: step_started.elapsed().as_millis(),
                request: Some(HttpRequestInfo {
                    method: method.to_string(),
                    url: full_url_for_report.clone(),
                    headers: rendered_headers.clone(),
                    body: request_body.clone(),
                }),
                response: None,
                assertions: Vec::new(),
            }
        }
    };
    let status = response.status().as_u16();
    let mut response_headers = BTreeMap::new();
    for (k, v) in response.headers() {
        response_headers.insert(
            k.to_string(),
            v.to_str().unwrap_or("<non-utf8>").to_string(),
        );
    }
    let body = response.text().await.unwrap_or_default();
    let (assertion_results, assertion_errors) = run_assertions(
        &step.assertions,
        status,
        &body,
        runtime_paths,
        &mut cache.schema_cache,
    );
    if assertion_errors.is_empty() {
        StepRunResult {
            name: step.name.clone(),
            status: "passed".to_string(),
            message: format!("OK ({})", status),
            response_status: Some(status),
            duration_ms: step_started.elapsed().as_millis(),
            request: Some(HttpRequestInfo {
                method: method.to_string(),
                url: full_url_for_report,
                headers: rendered_headers,
                body: request_body,
            }),
            response: Some(HttpResponseInfo {
                status,
                headers: response_headers,
                body,
            }),
            assertions: assertion_results,
        }
    } else {
        StepRunResult {
            name: step.name.clone(),
            status: "failed".to_string(),
            message: assertion_errors.join("; "),
            response_status: Some(status),
            duration_ms: step_started.elapsed().as_millis(),
            request: Some(HttpRequestInfo {
                method: method.to_string(),
                url: full_url_for_report,
                headers: rendered_headers,
                body: request_body,
            }),
            response: Some(HttpResponseInfo {
                status,
                headers: response_headers,
                body,
            }),
            assertions: assertion_results,
        }
    }
}

pub async fn run_test(
    test: &TestSpec,
    test_file: &Path,
    rel_file: String,
    base_url: &str,
    env_vars: &BTreeMap<String, Value>,
    runtime_paths: &RuntimePaths,
) -> TestRunResult {
    let started = Instant::now();
    let client = reqwest::Client::new();
    let mut cache = RuntimeCaches::default();
    let mut vars = env_vars.clone();
    let imported_vars = match collect_import_variables(&test.imports, runtime_paths, &mut cache.module_cache) {
        Ok(v) => v,
        Err(e) => {
            return TestRunResult {
                id: test.id.clone(),
                title: test.title.clone(),
                tags: test.tags.clone(),
                file: rel_file,
                status: "failed".to_string(),
                duration_ms: started.elapsed().as_millis(),
                errors: vec![e],
                steps: Vec::new(),
                setup_steps: Vec::new(),
                teardown_steps: Vec::new(),
            };
        }
    };
    for (k, v) in imported_vars {
        vars.insert(k, v);
    }
    for (k, v) in &test.variables {
        vars.insert(k.clone(), v.clone());
    }

    let mut step_results = Vec::new();
    let mut errors = Vec::new();

    run_step_group(
        &client,
        &vars,
        base_url,
        &test.setup,
        test_file,
        &test.imports,
        runtime_paths,
        &mut cache,
        &mut step_results,
        &mut errors,
    )
    .await;
    run_step_group(
        &client,
        &vars,
        base_url,
        &test.steps,
        test_file,
        &test.imports,
        runtime_paths,
        &mut cache,
        &mut step_results,
        &mut errors,
    )
    .await;
    run_step_group(
        &client,
        &vars,
        base_url,
        &test.cleanup,
        test_file,
        &test.imports,
        runtime_paths,
        &mut cache,
        &mut step_results,
        &mut errors,
    )
    .await;

    TestRunResult {
        id: test.id.clone(),
        title: test.title.clone(),
        tags: test.tags.clone(),
        file: rel_file,
        status: if errors.is_empty() {
            "passed".to_string()
        } else {
            "failed".to_string()
        },
        duration_ms: started.elapsed().as_millis(),
        errors,
        steps: step_results,
        setup_steps: Vec::new(),
        teardown_steps: Vec::new(),
    }
}

async fn run_step_group(
    client: &reqwest::Client,
    vars: &BTreeMap<String, Value>,
    base_url: &str,
    steps: &[Step],
    test_file: &Path,
    imports: &[ImportSpec],
    runtime_paths: &RuntimePaths,
    cache: &mut RuntimeCaches,
    step_results: &mut Vec<StepRunResult>,
    errors: &mut Vec<String>,
) {
    for step in steps {
        if step.step_type == "use" {
            let action = step.action.as_deref().map(str::trim).unwrap_or_default();
            if !action.is_empty() {
                match resolve_action_steps(step, imports, runtime_paths, &mut cache.module_cache) {
                    Ok(resolved_action) => {
                        let action_vars = match build_action_vars(vars, step, &resolved_action.required_properties) {
                            Ok(v) => v,
                            Err(msg) => {
                                errors.push(msg.clone());
                                step_results.push(StepRunResult {
                                    name: step.name.clone(),
                                    status: "failed".to_string(),
                                    message: msg,
                                    response_status: None,
                                    duration_ms: 0,
                                    request: None,
                                    response: None,
                                    assertions: Vec::new(),
                                });
                                continue;
                            }
                        };

                        for action_step in resolved_action.steps {
                            if action_step.step_type == "use" {
                                let msg = format!(
                                    "nested 'use' in module action is not supported yet (step '{}')",
                                    action_step.name
                                );
                                errors.push(msg.clone());
                                step_results.push(StepRunResult {
                                    name: action_step.name,
                                    status: "failed".to_string(),
                                    message: msg,
                                    response_status: None,
                                    duration_ms: 0,
                                    request: None,
                                    response: None,
                                    assertions: Vec::new(),
                                });
                                continue;
                            }
                            let result =
                                execute_api_step(client, &action_vars, base_url, &action_step, runtime_paths, cache)
                                    .await;
                            if result.status == "failed" {
                                errors.push(result.message.clone());
                            }
                            step_results.push(result);
                        }
                    }
                    Err(msg) => {
                        errors.push(msg.clone());
                        step_results.push(StepRunResult {
                            name: step.name.clone(),
                            status: "failed".to_string(),
                            message: msg,
                            response_status: None,
                            duration_ms: 0,
                            request: None,
                            response: None,
                            assertions: Vec::new(),
                        });
                    }
                }
                continue;
            }

            let ref_file = step.r#ref.clone().unwrap_or_default();
            let use_path = test_file.parent().unwrap_or_else(|| Path::new(".")).join(ref_file);
            match fs::read_to_string(&use_path) {
                Ok(content) => match parse_reusable_steps(&content, &use_path.to_string_lossy()) {
                    Ok(reusable_steps) => {
                        for reusable in reusable_steps {
                            let result =
                                execute_api_step(client, vars, base_url, &reusable, runtime_paths, cache).await;
                            if result.status == "failed" {
                                errors.push(result.message.clone());
                            }
                            step_results.push(result);
                        }
                    }
                    Err(e) => {
                        errors.push(e.clone());
                        step_results.push(StepRunResult {
                            name: step.name.clone(),
                            status: "failed".to_string(),
                            message: e,
                            response_status: None,
                            duration_ms: 0,
                            request: None,
                            response: None,
                            assertions: Vec::new(),
                        });
                    }
                },
                Err(e) => {
                    let msg = format!("failed to read reusable file {}: {}", use_path.display(), e);
                    errors.push(msg.clone());
                    step_results.push(StepRunResult {
                        name: step.name.clone(),
                        status: "failed".to_string(),
                        message: msg,
                        response_status: None,
                        duration_ms: 0,
                        request: None,
                        response: None,
                        assertions: Vec::new(),
                    });
                }
            }
        } else {
            let result = execute_api_step(client, vars, base_url, step, runtime_paths, cache).await;
            if result.status == "failed" {
                errors.push(result.message.clone());
            }
            step_results.push(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_tmp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("speq-cli-runner-{}-{}", name, suffix));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn schema_assertion_supports_ref_file() {
        let root = make_tmp_dir("schema-ref");
        let schemas_root = root.join("schemas");
        let modules_root = root.join("modules");
        fs::create_dir_all(&schemas_root).expect("schemas dir");
        fs::create_dir_all(&modules_root).expect("modules dir");
        fs::write(
            schemas_root.join("user.json"),
            r#"{"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}"#,
        )
        .expect("write schema");
        let runtime_paths = RuntimePaths {
            schemas_root,
            modules_root,
        };

        let assertions = vec![Assertion {
            assertion_type: "schema".to_string(),
            path: None,
            expected: None,
            value: None,
            r#ref: Some("user.json".to_string()),
            inline: None,
        }];
        let mut cache = HashMap::new();
        let (_details, errors) = run_assertions(&assertions, 200, r#"{"id":1}"#, &runtime_paths, &mut cache);
        assert!(errors.is_empty());
    }

    #[test]
    fn schema_assertion_inline_fails_with_reason() {
        let root = make_tmp_dir("schema-inline");
        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: root.join("modules"),
        };
        let assertions = vec![Assertion {
            assertion_type: "schema".to_string(),
            path: None,
            expected: None,
            value: None,
            r#ref: None,
            inline: Some(serde_json::json!({
                "type": "object",
                "required": ["name"]
            })),
        }];
        let mut cache = HashMap::new();
        let (_details, errors) = run_assertions(&assertions, 200, r#"{"id":1}"#, &runtime_paths, &mut cache);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("schema assertion failed"));
    }

    #[test]
    fn resolve_action_steps_uses_import_alias() {
        let root = make_tmp_dir("use-action");
        let schemas_root = root.join("schemas");
        let modules_root = root.join("modules");
        fs::create_dir_all(&schemas_root).expect("schemas dir");
        fs::create_dir_all(&modules_root).expect("modules dir");
        fs::write(
            modules_root.join("auth.yaml"),
            r#"
variables:
  token: "abc"
actions:
  login:
    - type: api
      name: "login request"
      method: GET
      url: "/status/200"
"#,
        )
        .expect("write module");
        let runtime_paths = RuntimePaths {
            schemas_root,
            modules_root,
        };
        let imports = vec![ImportSpec {
            module: "auth".to_string(),
            alias: Some("auth".to_string()),
        }];
        let step = Step {
            step_type: "use".to_string(),
            name: "login".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            r#ref: None,
            action: Some("auth.login".to_string()),
            properties: BTreeMap::new(),
            assertions: Vec::new(),
        };
        let mut cache = HashMap::new();
        let resolved = resolve_action_steps(&step, &imports, &runtime_paths, &mut cache).expect("resolve action");
        assert_eq!(resolved.steps.len(), 1);
        assert_eq!(resolved.steps[0].step_type, "api");
    }

    #[test]
    fn resolve_action_steps_supports_declared_properties() {
        let root = make_tmp_dir("use-action-properties");
        let schemas_root = root.join("schemas");
        let modules_root = root.join("modules");
        fs::create_dir_all(&schemas_root).expect("schemas dir");
        fs::create_dir_all(&modules_root).expect("modules dir");
        fs::write(
            modules_root.join("posts.yaml"),
            r#"
actions:
  getById:
    properties:
      - postId
    steps:
      - type: api
        name: "get post"
        method: GET
        url: "/posts/{{postId}}"
"#,
        )
        .expect("write module");
        let runtime_paths = RuntimePaths {
            schemas_root,
            modules_root,
        };
        let imports = vec![ImportSpec {
            module: "posts".to_string(),
            alias: Some("posts".to_string()),
        }];
        let step = Step {
            step_type: "use".to_string(),
            name: "get by id".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            r#ref: None,
            action: Some("posts.getById".to_string()),
            properties: BTreeMap::new(),
            assertions: Vec::new(),
        };
        let mut cache = HashMap::new();
        let resolved = resolve_action_steps(&step, &imports, &runtime_paths, &mut cache).expect("resolve action");
        assert_eq!(resolved.required_properties, vec!["postId".to_string()]);
    }
}
