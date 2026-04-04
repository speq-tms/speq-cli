use crate::cli::discovery::discover_speq_root;
use crate::cli::files::{collect_yaml_files, relative_unix};
use crate::manifest::read_manifest;
use crate::parser::parse_and_validate_test;
use crate::runner::{run_test, TestRunResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum ReportMode {
    All,
    Summary,
    Allure,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub speq_root_override: Option<String>,
    pub env_name: Option<String>,
    pub test_path: Option<String>,
    pub suite_path: Option<String>,
    pub tags: Vec<String>,
    pub report_mode: ReportMode,
    pub summary_output: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EnvYaml {
    #[serde(default, rename = "baseUrl")]
    base_url: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryTestRecord {
    pub id: String,
    pub status: String,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryTotals {
    pub passed: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryReport {
    pub status: String,
    pub started_at_ms: u128,
    pub duration_ms: u128,
    pub totals: SummaryTotals,
    pub tests: Vec<SummaryTestRecord>,
}

fn now_ms() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|x| x.as_millis())
        .map_err(|e| format!("internal: failed to read time: {e}"))
}

pub fn parse_tags_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

pub fn matches_tag_filter(test_tags: &[String], filter_tags: &[String]) -> bool {
    if filter_tags.is_empty() {
        return true;
    }
    test_tags
        .iter()
        .any(|tag| filter_tags.iter().any(|f| f == tag))
}

pub fn resolve_report_mode(raw: Option<String>) -> Result<ReportMode, String> {
    match raw.as_deref() {
        None => Ok(ReportMode::Allure),
        Some("all") => Ok(ReportMode::All),
        Some("summary") => Ok(ReportMode::Summary),
        Some("allure") => Ok(ReportMode::Allure),
        Some(other) => Err(format!(
            "unsupported report mode '{}', expected all|summary|allure",
            other
        )),
    }
}

fn read_env_vars(speq_root: &Path, env_name: &str) -> Result<(String, BTreeMap<String, serde_json::Value>), String> {
    let env_path = speq_root.join("environments").join(format!("{}.yaml", env_name));
    let content = fs::read_to_string(&env_path)
        .map_err(|e| format!("failed to read env file {}: {e}", env_path.display()))?;
    let parsed = serde_yaml::from_str::<EnvYaml>(&content)
        .map_err(|e| format!("invalid env yaml {}: {e}", env_path.display()))?;

    let base_url = parsed.base_url.unwrap_or_default();
    let mut vars = BTreeMap::new();
    for (k, v) in parsed.extra {
        let json_value = serde_json::to_value(v).unwrap_or(serde_json::Value::Null);
        vars.insert(k, json_value);
    }
    vars.insert("baseUrl".to_string(), serde_json::Value::String(base_url.clone()));
    Ok((base_url, vars))
}

pub fn collect_selected_files(
    speq_root: &Path,
    default_suites_dir: &str,
    test: Option<String>,
    suite: Option<String>,
) -> Result<Vec<PathBuf>, String> {
    if test.is_some() && suite.is_some() {
        return Err("use either --test or --suite, not both".to_string());
    }

    let resolve_path = |raw: String| {
        let p = PathBuf::from(&raw);
        if p.is_absolute() { p } else { speq_root.join(raw) }
    };

    let mut files = if let Some(test_path) = test {
        let p = resolve_path(test_path);
        if !p.is_file() {
            return Err(format!("test file does not exist: {}", p.display()));
        }
        vec![p]
    } else if let Some(suite_path) = suite {
        let p = resolve_path(suite_path);
        if !p.is_dir() {
            return Err(format!("suite path is not a directory: {}", p.display()));
        }
        collect_yaml_files(&p)
    } else {
        let suites_root = speq_root.join(default_suites_dir);
        if !suites_root.is_dir() {
            return Err(format!("suites directory does not exist: {}", suites_root.display()));
        }
        collect_yaml_files(&suites_root)
    };

    files.sort();
    if files.is_empty() {
        return Err("no tests selected for run".to_string());
    }
    Ok(files)
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(value).map_err(|e| format!("internal: failed to encode json: {e}"))?;
    fs::write(path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

pub fn write_allure_results(allure_dir: &Path, run_id: &str, results: &[TestRunResult]) -> Result<(), String> {
    fs::create_dir_all(allure_dir).map_err(|e| format!("failed to create {}: {e}", allure_dir.display()))?;
    for (idx, test) in results.iter().enumerate() {
        let payload = json!({
            "uuid": format!("{}-{}", run_id, idx + 1),
            "historyId": format!("{}::{}", test.id, test.file),
            "name": test.title,
            "status": test.status,
            "stage": "finished",
            "labels": [
              { "name": "framework", "value": "speq-cli" },
              { "name": "suite", "value": "api" }
            ],
            "steps": test.steps.iter().map(|s| json!({
              "name": s.name,
              "status": s.status,
              "stage": "finished",
              "statusDetails": { "message": s.message }
            })).collect::<Vec<_>>()
        });
        write_json(&allure_dir.join(format!("{}-result.json", idx + 1)), &payload)?;
    }
    write_json(
        &allure_dir.join("executor.json"),
        &json!({
          "name": "speq-cli",
          "type": "custom",
          "buildName": run_id
        }),
    )?;
    Ok(())
}

pub fn write_allure_from_summary(allure_dir: &Path, run_id: &str, summary: &SummaryReport) -> Result<(), String> {
    fs::create_dir_all(allure_dir).map_err(|e| format!("failed to create {}: {e}", allure_dir.display()))?;
    for (idx, test) in summary.tests.iter().enumerate() {
        let payload = json!({
            "uuid": format!("{}-{}", run_id, idx + 1),
            "historyId": test.id,
            "name": test.id,
            "status": test.status,
            "stage": "finished",
            "statusDetails": {
              "message": test.message.clone().unwrap_or_default()
            },
            "labels": [
              { "name": "framework", "value": "speq-cli" },
              { "name": "suite", "value": "api" }
            ]
        });
        write_json(&allure_dir.join(format!("{}-result.json", idx + 1)), &payload)?;
    }
    write_json(
        &allure_dir.join("executor.json"),
        &json!({
          "name": "speq-cli",
          "type": "custom",
          "buildName": run_id
        }),
    )?;
    Ok(())
}

pub fn build_run_options(
    speq_root_override: Option<String>,
    env_name: Option<String>,
    test_path: Option<String>,
    suite_path: Option<String>,
    tags_csv: Option<String>,
    report: Option<String>,
    summary_output: Option<String>,
) -> Result<RunOptions, String> {
    let report_mode = resolve_report_mode(report)?;
    Ok(RunOptions {
        speq_root_override,
        env_name,
        test_path,
        suite_path,
        tags: tags_csv.map(|x| parse_tags_csv(&x)).unwrap_or_default(),
        report_mode,
        summary_output,
    })
}

pub async fn command_run(options: RunOptions) -> Result<i32, String> {
    let discovered = discover_speq_root(options.speq_root_override)?;
    let manifest = read_manifest(&discovered.root)?;

    let env_name = options
        .env_name
        .unwrap_or_else(|| manifest.default_environment.clone());
    let (base_url, env_vars) = read_env_vars(&discovered.root, &env_name)?;
    if base_url.trim().is_empty() {
        return Err("baseUrl is required in selected environment file".to_string());
    }

    let files = collect_selected_files(
        &discovered.root,
        &manifest.suites_dir_or_default(),
        options.test_path,
        options.suite_path,
    )?;

    let started_at_ms = now_ms()?;
    let run_id = format!("run-{}", started_at_ms);
    let run_started = std::time::Instant::now();

    let mut results = Vec::new();
    for file in files {
        let content = fs::read_to_string(&file)
            .map_err(|e| format!("failed to read test file {}: {e}", file.display()))?;
        let parsed = parse_and_validate_test(&content, &file.to_string_lossy())?;

        if !matches_tag_filter(&parsed.tags, &options.tags) {
            continue;
        }

        let rel = relative_unix(&discovered.root, &file);
        let result = run_test(&parsed, &file, rel, &base_url, &env_vars).await;
        results.push(result);
    }

    if results.is_empty() {
        return Err("no tests matched selection/filter".to_string());
    }

    let passed = results.iter().filter(|x| x.status == "passed").count();
    let failed = results.len() - passed;
    let duration_ms = run_started.elapsed().as_millis();
    let status = if failed == 0 { "passed" } else { "failed" };

    let reports_root = discovered
        .root
        .join(manifest.reports_dir.clone().unwrap_or_else(|| "reports".to_string()));
    let summary_output = options.summary_output.clone();
    let summary_path = if let Some(raw_output) = summary_output {
        let p = PathBuf::from(raw_output);
        if p.is_absolute() {
            p
        } else {
            discovered.root.join(p)
        }
    } else {
        reports_root.join("results").join("summary.json")
    };
    let allure_dir = reports_root.join("allure");

    let summary_tests: Vec<SummaryTestRecord> = results
        .iter()
        .map(|r| SummaryTestRecord {
            id: r.id.clone(),
            status: r.status.clone(),
            duration_ms: r.duration_ms,
            message: r.errors.first().cloned(),
        })
        .collect();

    let summary_struct = SummaryReport {
        status: status.to_string(),
        started_at_ms: started_at_ms,
        duration_ms,
        totals: SummaryTotals {
            passed,
            failed,
            total: results.len(),
        },
        tests: summary_tests,
    };
    let summary_payload = serde_json::to_value(&summary_struct)
        .map_err(|e| format!("internal: failed to encode summary payload: {e}"))?;

    if matches!(options.report_mode, ReportMode::Allure) && options.summary_output.is_some() {
        return Err("`--output` is supported only with --report summary|all".to_string());
    }

    let (summary_generated, allure_generated) = match options.report_mode {
        ReportMode::Summary => {
            write_json(&summary_path, &summary_payload)?;
            (true, false)
        }
        ReportMode::Allure => {
            write_allure_results(&allure_dir, &run_id, &results)?;
            (false, true)
        }
        ReportMode::All => {
            write_json(&summary_path, &summary_payload)?;
            write_allure_results(&allure_dir, &run_id, &results)?;
            (true, true)
        }
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
          "ok": failed == 0,
          "status": status,
          "totals": { "passed": passed, "failed": failed, "total": results.len() },
          "reports": {
            "summary": if summary_generated { Some(summary_path.to_string_lossy().to_string()) } else { None::<String> },
            "allure": if allure_generated { Some(allure_dir.to_string_lossy().to_string()) } else { None::<String> }
          }
        }))
        .map_err(|e| format!("internal: failed to encode json: {e}"))?
    );

    Ok(if failed == 0 { 0 } else { 1 })
}

