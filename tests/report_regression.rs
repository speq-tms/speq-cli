use speq_cli::cli::report::{build_report_options, command_report, ReportFormat};
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
