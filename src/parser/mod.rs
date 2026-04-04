use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSpec {
    pub id: String,
    pub title: String,
    #[serde(default, alias = "markers")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub variables: BTreeMap<String, Value>,
    #[serde(default)]
    pub setup: Vec<Step>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub cleanup: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    #[serde(rename = "type")]
    pub step_type: String,
    pub name: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(default)]
    pub r#ref: Option<String>,
    #[serde(default, rename = "assert")]
    pub assertions: Vec<Assertion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assertion {
    #[serde(rename = "type")]
    pub assertion_type: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub expected: Option<Value>,
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReusableSpec {
    #[serde(default)]
    pub steps: Vec<Step>,
}

fn valid_http_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}

fn validate_assertion(assertion: &Assertion, file_path: &str, idx: usize) -> Result<(), String> {
    let allowed = ["status", "json", "contains", "notcontains", "exists", "regex"];
    if !allowed.contains(&assertion.assertion_type.as_str()) {
        return Err(format!(
            "unsupported assertion type '{}' in {} assert[{}]",
            assertion.assertion_type, file_path, idx
        ));
    }
    Ok(())
}

fn validate_step(step: &Step, file_path: &str, idx: usize) -> Result<(), String> {
    if step.name.trim().is_empty() {
        return Err(format!("step name is required in {} step[{}]", file_path, idx));
    }

    match step.step_type.as_str() {
        "api" => {
            if !valid_http_method(&step.method) {
                return Err(format!(
                    "unsupported HTTP method '{}' in {} step[{}]",
                    step.method, file_path, idx
                ));
            }
            if step.url.trim().is_empty() {
                return Err(format!("step url is required for api step in {} step[{}]", file_path, idx));
            }
        }
        "use" => {
            if step.r#ref.as_ref().map(|x| x.trim().is_empty()).unwrap_or(true) {
                return Err(format!("step ref is required for use step in {} step[{}]", file_path, idx));
            }
        }
        other => {
            return Err(format!(
                "unsupported step type '{}' in {} step[{}]",
                other, file_path, idx
            ));
        }
    }

    for (assert_idx, assertion) in step.assertions.iter().enumerate() {
        validate_assertion(assertion, file_path, assert_idx)?;
    }

    Ok(())
}

pub fn parse_and_validate_test(content: &str, file_path: &str) -> Result<TestSpec, String> {
    let parsed = serde_yaml::from_str::<TestSpec>(content)
        .map_err(|e| format!("YAML parse error in {}: {}", file_path, e))?;

    if parsed.id.trim().is_empty() {
        return Err(format!("'id' is required in {}", file_path));
    }
    if parsed.title.trim().is_empty() {
        return Err(format!("'title' is required in {}", file_path));
    }
    if parsed.steps.is_empty() && parsed.setup.is_empty() && parsed.cleanup.is_empty() {
        return Err(format!(
            "add at least one step in 'setup', 'steps' or 'cleanup' in {}",
            file_path
        ));
    }

    for (i, step) in parsed.setup.iter().enumerate() {
        validate_step(step, file_path, i)?;
    }
    for (i, step) in parsed.steps.iter().enumerate() {
        validate_step(step, file_path, i)?;
    }
    for (i, step) in parsed.cleanup.iter().enumerate() {
        validate_step(step, file_path, i)?;
    }

    Ok(parsed)
}

pub fn parse_reusable_steps(content: &str, file_path: &str) -> Result<Vec<Step>, String> {
    let parsed = serde_yaml::from_str::<ReusableSpec>(content)
        .map_err(|e| format!("YAML parse error in {}: {}", file_path, e))?;
    if parsed.steps.is_empty() {
        return Err(format!("'steps' is required in reusable file {}", file_path));
    }
    for (i, step) in parsed.steps.iter().enumerate() {
        if step.step_type == "use" {
            return Err(format!(
                "nested 'use' is not supported in reusable file {} step[{}]",
                file_path, i
            ));
        }
        validate_step(step, file_path, i)?;
    }
    Ok(parsed.steps)
}
