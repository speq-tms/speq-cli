use speq_cli::cli::report::{build_report_options, command_report, ReportFormat};
use speq_cli::cli::run::write_allure_results;
use speq_cli::runner::{AssertionRunResult, HttpRequestInfo, HttpResponseInfo, StepRunResult, TestRunResult};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_tmp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("speq-cli-test-{}-{}", name, suffix));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[test]
fn default_report_format_is_allure() {
    let opts = build_report_options(None, None, None).expect("report opts");
    assert!(matches!(opts.format, ReportFormat::Allure));
}

#[test]
fn report_generates_allure_from_summary() {
    let tmp = make_tmp_dir("allure-from-summary");
    let summary_dir = tmp.join("reports").join("results");
    fs::create_dir_all(&summary_dir).expect("summary dir");
    let summary_path = summary_dir.join("summary.json");
    fs::write(
        &summary_path,
        r#"{
  "status": "passed",
  "startedAtMs": 1,
  "durationMs": 5,
  "totals": { "passed": 1, "failed": 0, "total": 1 },
  "tests": [
    { "id": "smoke.health", "status": "passed", "durationMs": 5 }
  ]
}"#,
    )
    .expect("write summary");

    let options = build_report_options(
        Some(tmp.to_string_lossy().to_string()),
        Some("allure".to_string()),
        None,
    )
    .expect("opts");
    command_report(options).expect("command report");

    let allure_dir = tmp.join("reports").join("allure");
    assert!(allure_dir.join("1-result.json").is_file());
    assert!(allure_dir.join("executor.json").is_file());
}

#[test]
fn write_allure_results_writes_setup_and_teardown_to_container() {
    let tmp = make_tmp_dir("allure-hooks-container");
    let allure_dir = tmp.join("allure");
    let run_id = "run-hooks";
    let result = TestRunResult {
        id: "hooks.case".to_string(),
        title: "Hooks case".to_string(),
        tags: vec!["smoke".to_string()],
        file: "suites/users/smoke.yaml".to_string(),
        status: "passed".to_string(),
        duration_ms: 10,
        errors: Vec::new(),
        steps: vec![StepRunResult {
            name: "GET /users/1".to_string(),
            status: "passed".to_string(),
            message: "OK (200)".to_string(),
            response_status: Some(200),
            duration_ms: 5,
            request: None,
            response: None,
            assertions: Vec::new(),
        }],
        setup_steps: vec![StepRunResult {
            name: "[setup] beforeEach :: login".to_string(),
            status: "passed".to_string(),
            message: "OK (200)".to_string(),
            response_status: Some(200),
            duration_ms: 2,
            request: None,
            response: None,
            assertions: Vec::new(),
        }],
        teardown_steps: vec![StepRunResult {
            name: "[teardown] afterEach :: logout".to_string(),
            status: "passed".to_string(),
            message: "OK (200)".to_string(),
            response_status: Some(200),
            duration_ms: 2,
            request: None,
            response: None,
            assertions: Vec::new(),
        }],
    };

    write_allure_results(&allure_dir, run_id, &[result]).expect("write allure results");

    let container_raw = fs::read_to_string(allure_dir.join("1-container.json")).expect("read container");
    let container: Value = serde_json::from_str(&container_raw).expect("parse container");
    assert_eq!(container["befores"].as_array().map(|x| x.len()), Some(1));
    assert_eq!(container["afters"].as_array().map(|x| x.len()), Some(1));
    assert_eq!(container["befores"][0]["name"], "[setup] beforeEach :: login");
    assert_eq!(container["afters"][0]["name"], "[teardown] afterEach :: logout");
}

#[test]
fn write_allure_results_writes_request_response_and_assertions_attachments() {
    let tmp = make_tmp_dir("allure-step-attachments");
    let allure_dir = tmp.join("allure");
    let run_id = "run-attach";
    let step = StepRunResult {
        name: "GET /posts/1".to_string(),
        status: "passed".to_string(),
        message: "OK (200)".to_string(),
        response_status: Some(200),
        duration_ms: 8,
        request: Some(HttpRequestInfo {
            method: "GET".to_string(),
            url: "https://example.org/posts/1".to_string(),
            headers: std::collections::BTreeMap::from([("x-env".to_string(), "ci".to_string())]),
            body: None,
        }),
        response: Some(HttpResponseInfo {
            status: 200,
            headers: std::collections::BTreeMap::from([(
                "content-type".to_string(),
                "application/json".to_string(),
            )]),
            body: "{\"id\":1}".to_string(),
        }),
        assertions: vec![AssertionRunResult {
            assertion_type: "json".to_string(),
            status: "passed".to_string(),
            message: "json assertion passed at '$.id'".to_string(),
            path: Some("$.id".to_string()),
            expected: Some(json!(1)),
        }],
    };
    let result = TestRunResult {
        id: "attachments.case".to_string(),
        title: "Attachments case".to_string(),
        tags: vec!["smoke".to_string()],
        file: "suites/posts/smoke.yaml".to_string(),
        status: "passed".to_string(),
        duration_ms: 10,
        errors: Vec::new(),
        steps: vec![step],
        setup_steps: Vec::new(),
        teardown_steps: Vec::new(),
    };

    write_allure_results(&allure_dir, run_id, &[result]).expect("write allure results");

    let test_raw = fs::read_to_string(allure_dir.join("1-result.json")).expect("read result");
    let test_json: Value = serde_json::from_str(&test_raw).expect("parse result");
    let attachments = test_json["steps"][0]["attachments"]
        .as_array()
        .expect("attachments array");
    assert_eq!(attachments.len(), 3);
    assert_eq!(attachments[0]["name"], "request");
    assert_eq!(attachments[1]["name"], "response");
    assert_eq!(attachments[2]["name"], "assertions");
    let source = attachments[0]["source"].as_str().expect("source");
    assert!(allure_dir.join(source).is_file());
    let labels = test_json["labels"].as_array().expect("labels");
    assert!(labels.iter().any(|l| l["name"] == "suite" && l["value"] == "posts"));
    assert!(labels.iter().any(|l| l["name"] == "package" && l["value"] == "suites.posts"));
}
