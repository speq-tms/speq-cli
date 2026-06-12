use crate::fixtures::{load_fixture, materialize_fixture};
use crate::generator::{resolve_gen_values, resolve_gen_variables};
use crate::manifest::{BackoffStrategy, RetryConfig};
use crate::parser::{parse_reusable_steps, Assertion, ConditionConfig, ImportSpec, Step, TestSpec};
use jsonschema::JSONSchema;
use regex::Regex;
use reqwest::Method;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_used: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_duration_ms: Option<u64>,
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
    pub fixtures_root: PathBuf,
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
    #[serde(default)]
    returns: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
struct ResolvedAction {
    steps: Vec<Step>,
    required_properties: Vec<String>,
    returns: Option<HashMap<String, String>>,
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

fn render_template_in_value(value: &Value, vars: &BTreeMap<String, Value>) -> Value {
    match value {
        Value::String(s) => Value::String(render_template(s, vars)),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), render_template_in_value(v, vars));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| render_template_in_value(v, vars)).collect()),
        other => other.clone(),
    }
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

fn compute_delay_ms(config: &RetryConfig, attempt: u32) -> u64 {
    if config.delay_ms == 0 {
        return 0;
    }
    match config.backoff {
        BackoffStrategy::Fixed => config.delay_ms,
        BackoffStrategy::Exponential => {
            let exp = (attempt - 1).min(16) as u32;
            config.delay_ms.saturating_mul(1u64 << exp)
        }
    }
}

fn check_condition(condition: &ConditionConfig, body: &str) -> bool {
    match condition.condition_type.as_str() {
        "jsonpath" => {
            let Ok(json) = serde_json::from_str::<Value>(body) else {
                return false;
            };
            match json_path_get(&json, &condition.path) {
                Some(actual) => {
                    let actual_str = match actual {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    let equals_str = match &condition.equals {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    actual_str == equals_str
                }
                None => false,
            }
        }
        _ => false,
    }
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
            returns: None,
        },
        ModuleActionSpec::Detailed(action) => ResolvedAction {
            steps: action.steps,
            required_properties: action.properties,
            returns: action.returns,
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

fn resolve_returns_expression(
    expr: &str,
    id_responses: &HashMap<String, String>,
) -> Result<Value, String> {
    let rest = expr.strip_prefix("$steps.").ok_or_else(|| {
        format!(
            "module_return_resolution_error: expression '{}' must start with '$steps.'",
            expr
        )
    })?;
    let (step_id, body_path) = rest.split_once(".response.body.").ok_or_else(|| {
        format!(
            "module_return_resolution_error: expression '{}' must follow '$steps.<id>.response.body.<path>' format",
            expr
        )
    })?;
    let body_str = id_responses.get(step_id).ok_or_else(|| {
        format!(
            "module_return_resolution_error: step '{}' has no captured response (ensure the step has an 'id' field and succeeded)",
            step_id
        )
    })?;
    let json: Value = serde_json::from_str(body_str).map_err(|_| {
        format!(
            "module_return_resolution_error: step '{}' response is not valid JSON",
            step_id
        )
    })?;
    let json_path = format!("$.{}", body_path);
    json_path_get(&json, &json_path).cloned().ok_or_else(|| {
        format!(
            "module_return_resolution_error: path '{}' not found in step '{}' response",
            body_path, step_id
        )
    })
}

pub fn validate_module_content(content: &str, file_path: &str) -> Vec<String> {
    let module: ModuleSpec = match serde_yaml::from_str(content) {
        Ok(m) => m,
        Err(e) => return vec![format!("invalid module YAML {}: {}", file_path, e)],
    };
    let mut errors = Vec::new();
    for (action_name, action_spec) in &module.actions {
        if let ModuleActionSpec::Detailed(action) = action_spec {
            if let Some(returns_map) = &action.returns {
                let step_ids: std::collections::HashSet<&str> =
                    action.steps.iter().filter_map(|s| s.id.as_deref()).collect();
                for (field, expr) in returns_map {
                    if let Some(rest) = expr.strip_prefix("$steps.") {
                        if let Some((step_id, _)) = rest.split_once(".response.body.") {
                            if !step_ids.contains(step_id) {
                                errors.push(format!(
                                    "returns expression '{}' in action '{}' of {} references unknown step id '{}' (no step has id: {})",
                                    field, action_name, file_path, step_id, step_id
                                ));
                            }
                        } else {
                            errors.push(format!(
                                "returns expression '{}' in action '{}' of {} has invalid format (expected '$steps.<id>.response.body.<path>')",
                                field, action_name, file_path
                            ));
                        }
                    } else {
                        errors.push(format!(
                            "returns expression '{}' in action '{}' of {} must start with '$steps.'",
                            field, action_name, file_path
                        ));
                    }
                }
            }
        }
    }
    errors
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
    retry_config: Option<&RetryConfig>,
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
                attempts_used: None,
                wait_duration_ms: None,
            }
        }
    };

    let rendered_url = render_template(&step.url, vars);
    let full_url = if rendered_url.starts_with("http://") || rendered_url.starts_with("https://") {
        rendered_url
    } else {
        format!("{}{}", base_url.trim_end_matches('/'), rendered_url)
    };

    let mut rendered_headers = BTreeMap::new();
    for (k, v) in &step.headers {
        let rendered = render_template(v, vars);
        rendered_headers.insert(k.clone(), rendered);
    }

    let effective_body: Option<Value> = if let Some(bff) = &step.body_from_fixture {
        let fixture_path = runtime_paths.fixtures_root.join(&bff.r#ref);
        match load_fixture(&fixture_path) {
            Ok(fixture_cfg) => match materialize_fixture(&fixture_cfg, bff.overrides.as_ref()) {
                Ok(body) => Some(body),
                Err(e) => {
                    return StepRunResult {
                        name: step.name.clone(),
                        status: "failed".to_string(),
                        message: e,
                        response_status: None,
                        duration_ms: step_started.elapsed().as_millis(),
                        request: None,
                        response: None,
                        assertions: Vec::new(),
                        attempts_used: None,
                        wait_duration_ms: None,
                    }
                }
            },
            Err(e) => {
                return StepRunResult {
                    name: step.name.clone(),
                    status: "failed".to_string(),
                    message: e,
                    response_status: None,
                    duration_ms: step_started.elapsed().as_millis(),
                    request: None,
                    response: None,
                    assertions: Vec::new(),
                    attempts_used: None,
                    wait_duration_ms: None,
                }
            }
        }
    } else {
        step.body.clone()
    };

    let resolved_body: Option<Value> = if let Some(body) = &effective_body {
        let (resolved, gen_errors) = resolve_gen_values(body, "body");
        if !gen_errors.is_empty() {
            return StepRunResult {
                name: step.name.clone(),
                status: "failed".to_string(),
                message: gen_errors.join("; "),
                response_status: None,
                duration_ms: step_started.elapsed().as_millis(),
                request: None,
                response: None,
                assertions: Vec::new(),
                attempts_used: None,
                wait_duration_ms: None,
            };
        }
        Some(render_template_in_value(&resolved, vars))
    } else {
        None
    };

    let request_info = HttpRequestInfo {
        method: method.to_string(),
        url: full_url.clone(),
        headers: rendered_headers.clone(),
        body: effective_body.clone(),
    };

    let effective_retry = retry_config.filter(|c| c.enabled && c.max_attempts > 0);
    let max_attempts = effective_retry.map(|c| c.max_attempts).unwrap_or(1).max(1);

    let mut total_attempts: u32 = 0;
    let waiter_start = Instant::now();

    loop {
        // Retry loop for one HTTP call (with retry on transient failures)
        enum RequestOutcome {
            Success { status: u16, resp_headers: BTreeMap<String, String>, body: String },
            NetworkFail { message: String },
            StatusRetryExhausted { status: u16, resp_headers: BTreeMap<String, String>, body: String },
        }

        let mut retry_attempt: u32 = 0;

        let outcome: RequestOutcome = 'retry: loop {
            retry_attempt += 1;
            total_attempts += 1;

            let mut req = client.request(method.clone(), full_url.clone());
            for (k, v) in &rendered_headers {
                req = req.header(k, v);
            }
            if let Some(body) = &resolved_body {
                req = req.json(body);
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let mut resp_headers = BTreeMap::new();
                    for (k, v) in resp.headers() {
                        resp_headers.insert(k.to_string(), v.to_str().unwrap_or("<non-utf8>").to_string());
                    }
                    let body = resp.text().await.unwrap_or_default();

                    let should_retry_status = effective_retry
                        .map(|c| c.retry_on.status_codes.contains(&status))
                        .unwrap_or(false);

                    if should_retry_status && retry_attempt < max_attempts {
                        let delay = compute_delay_ms(effective_retry.unwrap(), retry_attempt);
                        if delay > 0 {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                        continue 'retry;
                    }

                    if should_retry_status {
                        break 'retry RequestOutcome::StatusRetryExhausted { status, resp_headers, body };
                    } else {
                        break 'retry RequestOutcome::Success { status, resp_headers, body };
                    }
                }
                Err(e) => {
                    let should_retry_network = effective_retry
                        .map(|c| c.retry_on.network_errors)
                        .unwrap_or(false);

                    if should_retry_network && retry_attempt < max_attempts {
                        let delay = compute_delay_ms(effective_retry.unwrap(), retry_attempt);
                        if delay > 0 {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                        continue 'retry;
                    }

                    let msg = if should_retry_network {
                        format!("retry_exhausted: request failed after {} attempt(s): {}", retry_attempt, e)
                    } else {
                        format!("request failed: {}", e)
                    };
                    break 'retry RequestOutcome::NetworkFail { message: msg };
                }
            }
        };

        match outcome {
            RequestOutcome::NetworkFail { message } => {
                return StepRunResult {
                    name: step.name.clone(),
                    status: "failed".to_string(),
                    message,
                    response_status: None,
                    duration_ms: step_started.elapsed().as_millis(),
                    request: Some(request_info),
                    response: None,
                    assertions: Vec::new(),
                    attempts_used: Some(total_attempts),
                    wait_duration_ms: None,
                };
            }
            RequestOutcome::StatusRetryExhausted { status, resp_headers, body } => {
                return StepRunResult {
                    name: step.name.clone(),
                    status: "failed".to_string(),
                    message: format!(
                        "retry_exhausted: request failed with status {} after {} attempt(s)",
                        status, total_attempts
                    ),
                    response_status: Some(status),
                    duration_ms: step_started.elapsed().as_millis(),
                    request: Some(request_info),
                    response: Some(HttpResponseInfo { status, headers: resp_headers, body }),
                    assertions: Vec::new(),
                    attempts_used: Some(total_attempts),
                    wait_duration_ms: None,
                };
            }
            RequestOutcome::Success { status, resp_headers, body } => {
                // Check condition (waiter)
                if let Some(condition) = &step.condition {
                    if check_condition(condition, &body) {
                        // Condition met — run assertions and return
                        let (assertion_results, assertion_errors) = run_assertions(
                            &step.assertions, status, &body, runtime_paths, &mut cache.schema_cache,
                        );
                        let wait_duration = waiter_start.elapsed().as_millis() as u64;
                        return StepRunResult {
                            name: step.name.clone(),
                            status: if assertion_errors.is_empty() { "passed".to_string() } else { "failed".to_string() },
                            message: if assertion_errors.is_empty() {
                                format!("OK ({}) — condition met", status)
                            } else {
                                assertion_errors.join("; ")
                            },
                            response_status: Some(status),
                            duration_ms: step_started.elapsed().as_millis(),
                            request: Some(request_info),
                            response: Some(HttpResponseInfo { status, headers: resp_headers, body }),
                            assertions: assertion_results,
                            attempts_used: Some(total_attempts),
                            wait_duration_ms: Some(wait_duration),
                        };
                    }

                    // Condition not met
                    if let Some(wait) = &condition.wait {
                        let elapsed_ms = waiter_start.elapsed().as_millis() as u64;
                        if elapsed_ms + wait.interval_ms > wait.timeout_ms {
                            return StepRunResult {
                                name: step.name.clone(),
                                status: "failed".to_string(),
                                message: format!(
                                    "wait_timeout: condition ({} {} = {}) not met within {}ms",
                                    condition.condition_type, condition.path, condition.equals, wait.timeout_ms
                                ),
                                response_status: Some(status),
                                duration_ms: step_started.elapsed().as_millis(),
                                request: Some(request_info),
                                response: Some(HttpResponseInfo { status, headers: resp_headers, body }),
                                assertions: Vec::new(),
                                attempts_used: Some(total_attempts),
                                wait_duration_ms: Some(elapsed_ms),
                            };
                        }
                        tokio::time::sleep(Duration::from_millis(wait.interval_ms)).await;
                        continue; // outer waiter loop
                    } else {
                        return StepRunResult {
                            name: step.name.clone(),
                            status: "failed".to_string(),
                            message: format!(
                                "condition not met: {} at {} (expected {})",
                                condition.condition_type, condition.path, condition.equals
                            ),
                            response_status: Some(status),
                            duration_ms: step_started.elapsed().as_millis(),
                            request: Some(request_info),
                            response: Some(HttpResponseInfo { status, headers: resp_headers, body }),
                            assertions: Vec::new(),
                            attempts_used: Some(total_attempts),
                            wait_duration_ms: None,
                        };
                    }
                }

                // No condition — run assertions and return
                let (assertion_results, assertion_errors) = run_assertions(
                    &step.assertions, status, &body, runtime_paths, &mut cache.schema_cache,
                );
                let attempts_field = if total_attempts > 1 { Some(total_attempts) } else { None };
                return if assertion_errors.is_empty() {
                    StepRunResult {
                        name: step.name.clone(),
                        status: "passed".to_string(),
                        message: format!("OK ({})", status),
                        response_status: Some(status),
                        duration_ms: step_started.elapsed().as_millis(),
                        request: Some(request_info),
                        response: Some(HttpResponseInfo { status, headers: resp_headers, body }),
                        assertions: assertion_results,
                        attempts_used: attempts_field,
                        wait_duration_ms: None,
                    }
                } else {
                    StepRunResult {
                        name: step.name.clone(),
                        status: "failed".to_string(),
                        message: assertion_errors.join("; "),
                        response_status: Some(status),
                        duration_ms: step_started.elapsed().as_millis(),
                        request: Some(request_info),
                        response: Some(HttpResponseInfo { status, headers: resp_headers, body }),
                        assertions: assertion_results,
                        attempts_used: attempts_field,
                        wait_duration_ms: None,
                    }
                };
            }
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
    retry_config: Option<&RetryConfig>,
) -> TestRunResult {
    let started = Instant::now();

    // ATDD: entire test is pending — skip without executing HTTP requests.
    if test.status.as_deref() == Some("pending") {
        return TestRunResult {
            id: test.id.clone(),
            title: test.title.clone(),
            tags: test.tags.clone(),
            file: rel_file,
            status: "pending".to_string(),
            duration_ms: started.elapsed().as_millis(),
            errors: Vec::new(),
            steps: Vec::new(),
            setup_steps: Vec::new(),
            teardown_steps: Vec::new(),
        };
    }

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
    match resolve_gen_variables(&test.variables) {
        Ok(resolved_vars) => {
            for (k, v) in resolved_vars {
                vars.insert(k, v);
            }
        }
        Err(gen_errors) => {
            return TestRunResult {
                id: test.id.clone(),
                title: test.title.clone(),
                tags: test.tags.clone(),
                file: rel_file,
                status: "failed".to_string(),
                duration_ms: started.elapsed().as_millis(),
                errors: gen_errors,
                steps: Vec::new(),
                setup_steps: Vec::new(),
                teardown_steps: Vec::new(),
            };
        }
    }

    let mut step_results = Vec::new();
    let mut errors = Vec::new();

    run_step_group(
        &client,
        &mut vars,
        base_url,
        &test.setup,
        test_file,
        &test.imports,
        runtime_paths,
        &mut cache,
        &mut step_results,
        &mut errors,
        retry_config,
    )
    .await;
    run_step_group(
        &client,
        &mut vars,
        base_url,
        &test.steps,
        test_file,
        &test.imports,
        runtime_paths,
        &mut cache,
        &mut step_results,
        &mut errors,
        retry_config,
    )
    .await;
    run_step_group(
        &client,
        &mut vars,
        base_url,
        &test.cleanup,
        test_file,
        &test.imports,
        runtime_paths,
        &mut cache,
        &mut step_results,
        &mut errors,
        retry_config,
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

fn step_references_pending(step: &Step, pending_step_names: &std::collections::HashSet<String>) -> bool {
    if pending_step_names.is_empty() {
        return false;
    }
    // Check all string fields that could contain {{ steps.<name>.response.* }} templates.
    let check_str = |s: &str| -> bool {
        pending_step_names.iter().any(|name| {
            let prefix = format!("steps.{}.", name);
            s.contains(&format!("{{{{ {} ", prefix.trim_end_matches('.')))
                || s.contains(&format!("{{{{{}", prefix))
                || s.contains(&format!("{{{{ {}", prefix))
        })
    };
    if check_str(&step.url) {
        return true;
    }
    for v in step.headers.values() {
        if check_str(v) {
            return true;
        }
    }
    if let Some(body) = &step.body {
        if check_str(&body.to_string()) {
            return true;
        }
    }
    false
}

async fn run_step_group(
    client: &reqwest::Client,
    vars: &mut BTreeMap<String, Value>,
    base_url: &str,
    steps: &[Step],
    test_file: &Path,
    imports: &[ImportSpec],
    runtime_paths: &RuntimePaths,
    cache: &mut RuntimeCaches,
    step_results: &mut Vec<StepRunResult>,
    errors: &mut Vec<String>,
    retry_config: Option<&RetryConfig>,
) {
    let mut pending_step_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for step in steps {
        // ATDD: skip steps marked as pending or that depend on a pending step's output.
        let is_pending = step.status.as_deref() == Some("pending")
            || step_references_pending(step, &pending_step_names);
        if is_pending {
            pending_step_names.insert(step.name.clone());
            step_results.push(StepRunResult {
                name: step.name.clone(),
                status: "pending".to_string(),
                message: "[ATDD: pending]".to_string(),
                response_status: None,
                duration_ms: 0,
                request: None,
                response: None,
                assertions: Vec::new(),
                attempts_used: None,
                wait_duration_ms: None,
            });
            continue;
        }
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
                                    attempts_used: None,
                                    wait_duration_ms: None,
                                });
                                continue;
                            }
                        };

                        let action_steps = resolved_action.steps;
                        let action_returns = resolved_action.returns;
                        let step_as = step.r#as.clone();

                        // Check for conflicts before executing any steps.
                        if action_returns.is_some() {
                            if let Some(as_key) = &step_as {
                                let prefix = format!("{}.", as_key);
                                let conflict = vars.contains_key(as_key.as_str())
                                    || vars.keys().any(|k| k.starts_with(&prefix));
                                if conflict {
                                    let msg = format!(
                                        "module_output_conflict: '{}' is already bound in context (step '{}')",
                                        as_key, step.name
                                    );
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
                                        attempts_used: None,
                                        wait_duration_ms: None,
                                    });
                                    continue;
                                }
                            }
                        }

                        let mut id_responses: HashMap<String, String> = HashMap::new();
                        let mut action_had_failure = false;

                        for action_step in &action_steps {
                            if action_step.step_type == "use" {
                                let msg = format!(
                                    "nested 'use' in module action is not supported yet (step '{}')",
                                    action_step.name
                                );
                                errors.push(msg.clone());
                                step_results.push(StepRunResult {
                                    name: action_step.name.clone(),
                                    status: "failed".to_string(),
                                    message: msg,
                                    response_status: None,
                                    duration_ms: 0,
                                    request: None,
                                    response: None,
                                    assertions: Vec::new(),
                                    attempts_used: None,
                                    wait_duration_ms: None,
                                });
                                action_had_failure = true;
                                continue;
                            }
                            let result =
                                execute_api_step(client, &action_vars, base_url, action_step, runtime_paths, cache, retry_config)
                                    .await;
                            if let (Some(step_id), Some(resp)) = (&action_step.id, &result.response) {
                                id_responses.insert(step_id.clone(), resp.body.clone());
                            }
                            if result.status == "failed" {
                                errors.push(result.message.clone());
                                action_had_failure = true;
                            }
                            step_results.push(result);
                        }

                        // Resolve and bind returns only when all steps passed.
                        if !action_had_failure {
                            if let (Some(returns_map), Some(as_key)) = (&action_returns, &step_as) {
                                for (field_name, expr) in returns_map {
                                    match resolve_returns_expression(expr, &id_responses) {
                                        Ok(value) => {
                                            vars.insert(format!("{}.{}", as_key, field_name), value);
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
                                                attempts_used: None,
                                                wait_duration_ms: None,
                                            });
                                            break;
                                        }
                                    }
                                }
                            }
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
                            attempts_used: None,
                            wait_duration_ms: None,
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
                                execute_api_step(client, vars, base_url, &reusable, runtime_paths, cache, retry_config).await;
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
                            attempts_used: None,
                            wait_duration_ms: None,
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
                        attempts_used: None,
                        wait_duration_ms: None,
                    });
                }
            }
        } else {
            let result = execute_api_step(client, vars, base_url, step, runtime_paths, cache, retry_config).await;
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
            fixtures_root: root.join("fixtures"),
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
            fixtures_root: root.join("fixtures"),
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
            fixtures_root: root.join("fixtures"),
        };
        let imports = vec![ImportSpec {
            module: "auth".to_string(),
            alias: Some("auth".to_string()),
        }];
        let step = Step {
            step_type: "use".to_string(),
            id: None,
            name: "login".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: Some("auth.login".to_string()),
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };
        let mut cache = HashMap::new();
        let resolved = resolve_action_steps(&step, &imports, &runtime_paths, &mut cache).expect("resolve action");
        assert_eq!(resolved.steps.len(), 1);
        assert_eq!(resolved.steps[0].step_type, "api");
    }

    #[tokio::test]
    async fn retry_succeeds_on_nth_attempt() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // Minimal HTTP server: returns 503 on first 2 calls, then 200
        let fail_times = 2u32;
        let call_count = std::sync::Arc::new(AtomicU32::new(0));
        let call_count_srv = call_count.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            for _ in 0..10u32 {
                let Ok((mut stream, _)) = listener.accept().await else { break };
                let n = call_count_srv.fetch_add(1, Ordering::SeqCst);
                let (status, body) = if n < fail_times {
                    (503u16, r#"{"error":"unavailable"}"#)
                } else {
                    (200u16, r#"{"ok":true}"#)
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            }
        });

        let root = make_tmp_dir("retry-ok");
        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: root.join("modules"),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let vars = BTreeMap::new();
        let retry = crate::manifest::RetryConfig {
            enabled: true,
            max_attempts: 5,
            delay_ms: 0,
            backoff: crate::manifest::BackoffStrategy::Fixed,
            retry_on: crate::manifest::RetryOn {
                network_errors: false,
                status_codes: vec![503],
            },
        };
        let step = crate::parser::Step {
            step_type: "api".to_string(),
            id: None,
            name: "test retry".to_string(),
            method: "GET".to_string(),
            url: format!("http://127.0.0.1:{}/retry-ok", port),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: None,
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };
        let mut cache = RuntimeCaches::default();
        let result = execute_api_step(&client, &vars, "", &step, &runtime_paths, &mut cache, Some(&retry)).await;
        assert_eq!(result.status, "passed", "expected passed, got: {}", result.message);
        // 2 failures + 1 success = 3 total attempts
        assert_eq!(result.attempts_used, Some(3));
    }

    #[tokio::test]
    async fn retry_exhausted_returns_error() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        let _m = server.mock_async(|when, then| {
            when.method(GET).path("/always-fail");
            then.status(429).body("rate limited");
        })
        .await;

        let root = make_tmp_dir("retry-exhausted");
        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: root.join("modules"),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let vars = BTreeMap::new();
        let retry = crate::manifest::RetryConfig {
            enabled: true,
            max_attempts: 3,
            delay_ms: 0,
            backoff: crate::manifest::BackoffStrategy::Fixed,
            retry_on: crate::manifest::RetryOn {
                network_errors: false,
                status_codes: vec![429],
            },
        };
        let step = crate::parser::Step {
            step_type: "api".to_string(),
            id: None,
            name: "exhausted".to_string(),
            method: "GET".to_string(),
            url: format!("{}/always-fail", server.base_url()),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: None,
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };
        let mut cache = RuntimeCaches::default();
        let result = execute_api_step(&client, &vars, "", &step, &runtime_paths, &mut cache, Some(&retry)).await;
        assert_eq!(result.status, "failed");
        assert!(result.message.contains("retry_exhausted"), "expected retry_exhausted, got: {}", result.message);
        assert_eq!(result.attempts_used, Some(3));
    }

    #[tokio::test]
    async fn waiter_timeout_returns_error() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        let _m = server.mock_async(|when, then| {
            when.method(GET).path("/status");
            then.status(200).body(r#"{"state":"pending"}"#);
        })
        .await;

        let root = make_tmp_dir("waiter-timeout");
        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: root.join("modules"),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let vars = BTreeMap::new();
        let step = crate::parser::Step {
            step_type: "api".to_string(),
            id: None,
            name: "waiter".to_string(),
            method: "GET".to_string(),
            url: format!("{}/status", server.base_url()),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: None,
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: Some(crate::parser::ConditionConfig {
                condition_type: "jsonpath".to_string(),
                path: "$.state".to_string(),
                equals: serde_json::json!("done"),
                wait: Some(crate::parser::WaitConfig {
                    timeout_ms: 200,
                    interval_ms: 50,
                }),
            }),
            status: None,
        };
        let mut cache = RuntimeCaches::default();
        let result = execute_api_step(&client, &vars, "", &step, &runtime_paths, &mut cache, None).await;
        assert_eq!(result.status, "failed");
        assert!(result.message.contains("wait_timeout"), "expected wait_timeout, got: {}", result.message);
        assert!(result.wait_duration_ms.is_some());
    }

    #[tokio::test]
    async fn waiter_succeeds_when_condition_met() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        let _m = server.mock_async(|when, then| {
            when.method(GET).path("/done");
            then.status(200).body(r#"{"state":"done"}"#);
        })
        .await;

        let root = make_tmp_dir("waiter-ok");
        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: root.join("modules"),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let vars = BTreeMap::new();
        let step = crate::parser::Step {
            step_type: "api".to_string(),
            id: None,
            name: "wait-done".to_string(),
            method: "GET".to_string(),
            url: format!("{}/done", server.base_url()),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: None,
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: Some(crate::parser::ConditionConfig {
                condition_type: "jsonpath".to_string(),
                path: "$.state".to_string(),
                equals: serde_json::json!("done"),
                wait: Some(crate::parser::WaitConfig {
                    timeout_ms: 5000,
                    interval_ms: 100,
                }),
            }),
            status: None,
        };
        let mut cache = RuntimeCaches::default();
        let result = execute_api_step(&client, &vars, "", &step, &runtime_paths, &mut cache, None).await;
        assert_eq!(result.status, "passed", "expected passed, got: {}", result.message);
        assert!(result.wait_duration_ms.is_some());
    }

    #[tokio::test]
    async fn no_retry_no_condition_passes_normally() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        let _m = server.mock_async(|when, then| {
            when.method(GET).path("/ok");
            then.status(200).body(r#"{"result":"ok"}"#);
        })
        .await;

        let root = make_tmp_dir("no-retry");
        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: root.join("modules"),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let vars = BTreeMap::new();
        let step = crate::parser::Step {
            step_type: "api".to_string(),
            id: None,
            name: "plain".to_string(),
            method: "GET".to_string(),
            url: format!("{}/ok", server.base_url()),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: None,
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };
        let mut cache = RuntimeCaches::default();
        let result = execute_api_step(&client, &vars, "", &step, &runtime_paths, &mut cache, None).await;
        assert_eq!(result.status, "passed", "expected passed, got: {}", result.message);
        assert_eq!(result.attempts_used, None);
        assert_eq!(result.wait_duration_ms, None);
    }

    #[test]
    fn resolve_returns_expression_resolves_nested_path() {
        let mut id_responses = HashMap::new();
        id_responses.insert(
            "login_call".to_string(),
            r#"{"access_token":"tok123","user":{"id":42}}"#.to_string(),
        );
        let token =
            resolve_returns_expression("$steps.login_call.response.body.access_token", &id_responses)
                .expect("resolve token");
        assert_eq!(token, serde_json::json!("tok123"));

        let user_id =
            resolve_returns_expression("$steps.login_call.response.body.user.id", &id_responses)
                .expect("resolve user id");
        assert_eq!(user_id, serde_json::json!(42));
    }

    #[test]
    fn resolve_returns_expression_missing_step_id_is_error() {
        let id_responses: HashMap<String, String> = HashMap::new();
        let result =
            resolve_returns_expression("$steps.nonexistent.response.body.token", &id_responses);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("module_return_resolution_error"),
            "expected module_return_resolution_error"
        );
    }

    #[test]
    fn resolve_returns_expression_bad_format_is_error() {
        let id_responses: HashMap<String, String> = HashMap::new();
        let result = resolve_returns_expression("bad_expression_no_prefix", &id_responses);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("module_return_resolution_error"));
    }

    #[test]
    fn resolve_returns_expression_missing_path_is_error() {
        let mut id_responses = HashMap::new();
        id_responses.insert("my_step".to_string(), r#"{"name":"test"}"#.to_string());
        let result =
            resolve_returns_expression("$steps.my_step.response.body.nonexistent.deep", &id_responses);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("module_return_resolution_error"));
    }

    #[tokio::test]
    async fn module_returns_auth_flow_binds_token() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/auth/login");
                then.status(200)
                    .body(r#"{"access_token":"tok123","user":{"id":42}}"#);
            })
            .await;

        let root = make_tmp_dir("module-returns-auth");
        let modules_root = root.join("modules");
        fs::create_dir_all(&modules_root).expect("modules dir");

        fs::write(
            modules_root.join("auth.yaml"),
            format!(
                r#"
actions:
  login:
    properties:
      - user
      - pass
    steps:
      - type: api
        id: login_call
        name: "login api call"
        method: POST
        url: "{}/auth/login"
    returns:
      token: "$steps.login_call.response.body.access_token"
      userId: "$steps.login_call.response.body.user.id"
"#,
                server.base_url()
            ),
        )
        .expect("write module");

        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: modules_root.clone(),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let mut vars: BTreeMap<String, Value> = BTreeMap::new();
        let imports = vec![ImportSpec {
            module: "auth".to_string(),
            alias: Some("auth".to_string()),
        }];

        let mut props = BTreeMap::new();
        props.insert("user".to_string(), serde_json::json!("john"));
        props.insert("pass".to_string(), serde_json::json!("secret"));

        let use_step = Step {
            step_type: "use".to_string(),
            id: None,
            name: "auth login".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: Some("auth.login".to_string()),
            properties: props,
            r#as: Some("login".to_string()),
            assertions: Vec::new(),
            condition: None,
            status: None,
        };

        let mut step_results = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut cache = RuntimeCaches::default();

        run_step_group(
            &client,
            &mut vars,
            "",
            &[use_step],
            std::path::Path::new("/tmp/test.yaml"),
            &imports,
            &runtime_paths,
            &mut cache,
            &mut step_results,
            &mut errors,
            None,
        )
        .await;

        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
        assert_eq!(vars.get("login.token"), Some(&serde_json::json!("tok123")));
        assert_eq!(vars.get("login.userId"), Some(&serde_json::json!(42)));
    }

    #[tokio::test]
    async fn module_returns_create_entity_reuse_id() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/entities");
                then.status(201).body(r#"{"id":"ent-456","name":"test"}"#);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/entities/ent-456");
                then.status(200).body(r#"{"id":"ent-456","status":"active"}"#);
            })
            .await;

        let root = make_tmp_dir("module-returns-entity");
        let modules_root = root.join("modules");
        fs::create_dir_all(&modules_root).expect("modules dir");

        fs::write(
            modules_root.join("entities.yaml"),
            format!(
                r#"
actions:
  create:
    steps:
      - type: api
        id: create_call
        name: "create entity"
        method: POST
        url: "{}/entities"
    returns:
      id: "$steps.create_call.response.body.id"
"#,
                server.base_url()
            ),
        )
        .expect("write module");

        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: modules_root.clone(),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let mut vars: BTreeMap<String, Value> = BTreeMap::new();
        let imports = vec![ImportSpec {
            module: "entities".to_string(),
            alias: Some("entities".to_string()),
        }];

        let create_step = Step {
            step_type: "use".to_string(),
            id: None,
            name: "create entity".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: Some("entities.create".to_string()),
            properties: BTreeMap::new(),
            r#as: Some("entity".to_string()),
            assertions: Vec::new(),
            condition: None,
            status: None,
        };

        let get_step = Step {
            step_type: "api".to_string(),
            id: None,
            name: "get created entity".to_string(),
            method: "GET".to_string(),
            url: format!("{}/entities/{{{{entity.id}}}}", server.base_url()),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: None,
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };

        let mut step_results = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut cache = RuntimeCaches::default();

        run_step_group(
            &client,
            &mut vars,
            "",
            &[create_step, get_step],
            std::path::Path::new("/tmp/test.yaml"),
            &imports,
            &runtime_paths,
            &mut cache,
            &mut step_results,
            &mut errors,
            None,
        )
        .await;

        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
        assert_eq!(vars.get("entity.id"), Some(&serde_json::json!("ent-456")));
        let get_result = step_results.iter().find(|r| r.name == "get created entity").unwrap();
        assert_eq!(get_result.status, "passed", "get step should pass: {}", get_result.message);
    }

    #[tokio::test]
    async fn module_use_without_returns_works_unchanged() {
        use httpmock::prelude::*;

        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(GET).path("/status");
                then.status(200).body(r#"{"ok":true}"#);
            })
            .await;

        let root = make_tmp_dir("module-no-returns");
        let modules_root = root.join("modules");
        fs::create_dir_all(&modules_root).expect("modules dir");

        fs::write(
            modules_root.join("health.yaml"),
            format!(
                r#"
actions:
  check:
    steps:
      - type: api
        name: "health check"
        method: GET
        url: "{}/status"
"#,
                server.base_url()
            ),
        )
        .expect("write module");

        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: modules_root.clone(),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let mut vars: BTreeMap<String, Value> = BTreeMap::new();
        let imports = vec![ImportSpec {
            module: "health".to_string(),
            alias: Some("health".to_string()),
        }];

        let use_step = Step {
            step_type: "use".to_string(),
            id: None,
            name: "check health".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: Some("health.check".to_string()),
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };

        let mut step_results = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut cache = RuntimeCaches::default();

        run_step_group(
            &client,
            &mut vars,
            "",
            &[use_step],
            std::path::Path::new("/tmp/test.yaml"),
            &imports,
            &runtime_paths,
            &mut cache,
            &mut step_results,
            &mut errors,
            None,
        )
        .await;

        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
        assert!(vars.is_empty(), "expected no vars to be bound after legacy use");
    }

    #[tokio::test]
    async fn module_returns_as_conflict_produces_error() {
        let root = make_tmp_dir("module-returns-conflict");
        let modules_root = root.join("modules");
        fs::create_dir_all(&modules_root).expect("modules dir");

        fs::write(
            modules_root.join("auth.yaml"),
            r#"
actions:
  login:
    steps:
      - type: api
        id: login_call
        name: "login"
        method: GET
        url: "/login"
    returns:
      token: "$steps.login_call.response.body.token"
"#,
        )
        .expect("write module");

        let runtime_paths = RuntimePaths {
            schemas_root: root.join("schemas"),
            modules_root: modules_root.clone(),
            fixtures_root: root.join("fixtures"),
        };
        let client = reqwest::Client::new();
        let mut vars: BTreeMap<String, Value> = BTreeMap::new();
        vars.insert("login.token".to_string(), serde_json::json!("existing"));

        let imports = vec![ImportSpec {
            module: "auth".to_string(),
            alias: Some("auth".to_string()),
        }];

        let use_step = Step {
            step_type: "use".to_string(),
            id: None,
            name: "auth login".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: Some("auth.login".to_string()),
            properties: BTreeMap::new(),
            r#as: Some("login".to_string()),
            assertions: Vec::new(),
            condition: None,
            status: None,
        };

        let mut step_results = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        let mut cache = RuntimeCaches::default();

        run_step_group(
            &client,
            &mut vars,
            "",
            &[use_step],
            std::path::Path::new("/tmp/test.yaml"),
            &imports,
            &runtime_paths,
            &mut cache,
            &mut step_results,
            &mut errors,
            None,
        )
        .await;

        assert!(!errors.is_empty(), "expected a conflict error");
        assert!(
            errors[0].contains("module_output_conflict"),
            "expected module_output_conflict, got: {}",
            errors[0]
        );
        assert_eq!(
            vars.get("login.token"),
            Some(&serde_json::json!("existing")),
            "existing value must not be overwritten"
        );
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
            fixtures_root: root.join("fixtures"),
        };
        let imports = vec![ImportSpec {
            module: "posts".to_string(),
            alias: Some("posts".to_string()),
        }];
        let step = Step {
            step_type: "use".to_string(),
            id: None,
            name: "get by id".to_string(),
            method: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            body: None,
            body_from_fixture: None,
            r#ref: None,
            action: Some("posts.getById".to_string()),
            properties: BTreeMap::new(),
            r#as: None,
            assertions: Vec::new(),
            condition: None,
            status: None,
        };
        let mut cache = HashMap::new();
        let resolved = resolve_action_steps(&step, &imports, &runtime_paths, &mut cache).expect("resolve action");
        assert_eq!(resolved.required_properties, vec!["postId".to_string()]);
    }
}
