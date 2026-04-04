use crate::parser::{parse_reusable_steps, Assertion, Step, TestSpec};
use regex::Regex;
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRunResult {
    pub name: String,
    pub status: String,
    pub message: String,
    pub response_status: Option<u16>,
    pub duration_ms: u128,
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

fn run_assertions(assertions: &[Assertion], status: u16, body: &str) -> Vec<String> {
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
                    errors.push(format!(
                        "status assertion failed: expected {:?}, got {}",
                        expected, status
                    ));
                }
            }
            "contains" => {
                let needle = assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !body.contains(needle) {
                    errors.push(format!("contains assertion failed: '{}' not found", needle));
                }
            }
            "notcontains" => {
                let needle = assertion
                    .expected
                    .as_ref()
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if body.contains(needle) {
                    errors.push(format!("notcontains assertion failed: '{}' found", needle));
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
                            errors.push(format!("regex assertion failed: '{}' not matched", pattern));
                        }
                    }
                    Err(e) => errors.push(format!("invalid regex '{}': {}", pattern, e)),
                }
            }
            "exists" => {
                let path = assertion.path.as_deref().unwrap_or("$");
                match &parsed_json {
                    Some(json) if json_path_get(json, path).is_some() => {}
                    Some(_) => errors.push(format!("exists assertion failed: path '{}' not found", path)),
                    None => errors.push("exists assertion failed: response is not json".to_string()),
                }
            }
            "json" => {
                let path = assertion.path.as_deref().unwrap_or("$");
                let expected = assertion.expected.clone().unwrap_or(Value::Null);
                match &parsed_json {
                    Some(json) => match json_path_get(json, path) {
                        Some(actual) if actual == &expected => {}
                        Some(actual) => errors.push(format!(
                            "json assertion failed at '{}': expected {}, got {}",
                            path, expected, actual
                        )),
                        None => errors.push(format!("json assertion failed: path '{}' not found", path)),
                    },
                    None => errors.push("json assertion failed: response is not json".to_string()),
                }
            }
            other => errors.push(format!("unsupported assertion type in runtime: {}", other)),
        }
    }
    errors
}

async fn execute_api_step(
    client: &reqwest::Client,
    vars: &BTreeMap<String, Value>,
    base_url: &str,
    step: &Step,
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
            }
        }
    };

    let rendered_url = render_template(&step.url, vars);
    let full_url = if rendered_url.starts_with("http://") || rendered_url.starts_with("https://") {
        rendered_url
    } else {
        format!("{}{}", base_url.trim_end_matches('/'), rendered_url)
    };

    let mut req = client.request(method, full_url);
    for (k, v) in &step.headers {
        req = req.header(k, render_template(v, vars));
    }
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
            }
        }
    };
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let assertion_errors = run_assertions(&step.assertions, status, &body);
    if assertion_errors.is_empty() {
        StepRunResult {
            name: step.name.clone(),
            status: "passed".to_string(),
            message: format!("OK ({})", status),
            response_status: Some(status),
            duration_ms: step_started.elapsed().as_millis(),
        }
    } else {
        StepRunResult {
            name: step.name.clone(),
            status: "failed".to_string(),
            message: assertion_errors.join("; "),
            response_status: Some(status),
            duration_ms: step_started.elapsed().as_millis(),
        }
    }
}

pub async fn run_test(
    test: &TestSpec,
    test_file: &Path,
    rel_file: String,
    base_url: &str,
    env_vars: &BTreeMap<String, Value>,
) -> TestRunResult {
    let started = Instant::now();
    let client = reqwest::Client::new();
    let mut vars = env_vars.clone();
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
    }
}

async fn run_step_group(
    client: &reqwest::Client,
    vars: &BTreeMap<String, Value>,
    base_url: &str,
    steps: &[Step],
    test_file: &Path,
    step_results: &mut Vec<StepRunResult>,
    errors: &mut Vec<String>,
) {
    for step in steps {
        if step.step_type == "use" {
            let ref_file = step.r#ref.clone().unwrap_or_default();
            let use_path = test_file.parent().unwrap_or_else(|| Path::new(".")).join(ref_file);
            match fs::read_to_string(&use_path) {
                Ok(content) => match parse_reusable_steps(&content, &use_path.to_string_lossy()) {
                    Ok(reusable_steps) => {
                        for reusable in reusable_steps {
                            let result = execute_api_step(client, vars, base_url, &reusable).await;
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
                    });
                }
            }
        } else {
            let result = execute_api_step(client, vars, base_url, step).await;
            if result.status == "failed" {
                errors.push(result.message.clone());
            }
            step_results.push(result);
        }
    }
}
