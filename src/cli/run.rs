use crate::cli::discovery::discover_speq_root;
use crate::cli::files::{collect_suite_init_files, collect_yaml_files, relative_unix};
use crate::manifest::read_manifest;
use crate::parser::{parse_and_validate_suite_init, parse_and_validate_test, Step, SuiteInitSpec, TestSpec};
use crate::runner::{run_test, StepRunResult, TestRunResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};
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

#[derive(Debug, Clone)]
struct SuiteExecutionContext {
    suite_key: String,
    suite_variables: BTreeMap<String, serde_json::Value>,
    before_all: Vec<Step>,
    before_each: Vec<Step>,
    after_each: Vec<Step>,
    after_all: Vec<Step>,
}

#[derive(Debug, Clone)]
struct HookExecutionResult {
    status: String,
    message: String,
    steps: Vec<StepRunResult>,
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

fn suite_key_for_test(suites_root: &Path, test_file: &Path) -> String {
    let parent = test_file.parent().unwrap_or(suites_root);
    relative_unix(suites_root, parent)
}

fn collect_suite_init_chain(suites_root: &Path, test_file: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = test_file.parent();
    while let Some(dir) = current {
        if !dir.starts_with(suites_root) {
            break;
        }
        let init_yaml = dir.join("init.yaml");
        let init_yml = dir.join("init.yml");
        if init_yaml.is_file() {
            chain.push(init_yaml);
        } else if init_yml.is_file() {
            chain.push(init_yml);
        }
        if dir == suites_root {
            break;
        }
        current = dir.parent();
    }
    chain.reverse();
    chain
}

fn load_suite_init_cache(suites_root: &Path) -> Result<HashMap<PathBuf, SuiteInitSpec>, String> {
    let mut cache = HashMap::new();
    for init_file in collect_suite_init_files(suites_root) {
        let content = fs::read_to_string(&init_file)
            .map_err(|e| format!("failed to read suite init {}: {e}", init_file.display()))?;
        let parsed = parse_and_validate_suite_init(&content, &init_file.to_string_lossy())?;
        cache.insert(init_file, parsed);
    }
    Ok(cache)
}

fn build_suite_execution_context(
    suites_root: &Path,
    test_file: &Path,
    init_cache: &HashMap<PathBuf, SuiteInitSpec>,
) -> SuiteExecutionContext {
    let chain = collect_suite_init_chain(suites_root, test_file);
    let mut variables = BTreeMap::new();
    let mut before_each = Vec::new();
    let mut after_each = Vec::new();

    for init_path in &chain {
        if let Some(spec) = init_cache.get(init_path) {
            for (k, v) in &spec.suite.variables {
                variables.insert(k.clone(), v.clone());
            }
            before_each.extend(spec.suite.before_each.clone());
            after_each.extend(spec.suite.after_each.clone());
        }
    }

    let (before_all, after_all) = if let Some(last) = chain.last() {
        if let Some(spec) = init_cache.get(last) {
            (spec.suite.before_all.clone(), spec.suite.after_all.clone())
        } else {
            (Vec::new(), Vec::new())
        }
    } else {
        (Vec::new(), Vec::new())
    };

    after_each.reverse();

    SuiteExecutionContext {
        suite_key: suite_key_for_test(suites_root, test_file),
        suite_variables: variables,
        before_all,
        before_each,
        after_each,
        after_all,
    }
}

async fn run_hook_steps(
    hook_name: &str,
    hook_scope: &str,
    steps: &[Step],
    test_file: &Path,
    suite_key: &str,
    base_url: &str,
    vars: &BTreeMap<String, serde_json::Value>,
) -> Option<HookExecutionResult> {
    if steps.is_empty() {
        return None;
    }

    let hook_spec = TestSpec {
        id: format!("__hook.{}.{}", suite_key, hook_name),
        title: format!("{} hook ({})", hook_name, suite_key),
        tags: Vec::new(),
        variables: BTreeMap::new(),
        setup: Vec::new(),
        steps: steps.to_vec(),
        cleanup: Vec::new(),
    };
    let result = run_test(
        &hook_spec,
        test_file,
        format!("{}#{}", suite_key, hook_name),
        base_url,
        vars,
    )
    .await;
    let prefixed_steps = result
        .steps
        .into_iter()
        .map(|mut s| {
            s.name = format!("[{}] {} :: {}", hook_scope, hook_name, s.name);
            s
        })
        .collect::<Vec<_>>();
    Some(HookExecutionResult {
        status: result.status,
        message: if result.errors.is_empty() {
            "hook completed".to_string()
        } else {
            result.errors.join("; ")
        },
        steps: prefixed_steps,
    })
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

fn write_attachment_text(allure_dir: &Path, source: &str, content: &str) -> Result<(), String> {
    fs::write(allure_dir.join(source), content)
        .map_err(|e| format!("failed to write attachment {}: {e}", allure_dir.join(source).display()))
}

fn build_suite_labels(test_file: &str) -> Vec<serde_json::Value> {
    let normalized = test_file.replace('\\', "/");
    let parts = normalized.split('/').collect::<Vec<_>>();
    let mut dirs = if let Some(pos) = parts.iter().position(|x| *x == "suites") {
        parts[pos + 1..parts.len().saturating_sub(1)].to_vec()
    } else {
        parts[..parts.len().saturating_sub(1)].to_vec()
    };
    dirs.retain(|x| !x.is_empty());

    let mut labels = vec![json!({ "name": "framework", "value": "speq-cli" })];
    let package_value = if dirs.is_empty() {
        "suites".to_string()
    } else {
        format!("suites.{}", dirs.join("."))
    };
    labels.push(json!({
      "name": "package",
      "value": package_value
    }));

    match dirs.len() {
        0 => {
            labels.push(json!({ "name": "suite", "value": "suites" }));
        }
        1 => {
            labels.push(json!({ "name": "suite", "value": dirs[0] }));
        }
        2 => {
            labels.push(json!({ "name": "parentSuite", "value": dirs[0] }));
            labels.push(json!({ "name": "suite", "value": dirs[1] }));
        }
        _ => {
            labels.push(json!({ "name": "parentSuite", "value": dirs[0] }));
            labels.push(json!({ "name": "suite", "value": dirs[1] }));
            labels.push(json!({ "name": "subSuite", "value": dirs[2..].join("/") }));
        }
    }
    labels
}

fn encode_step_with_attachments(
    allure_dir: &Path,
    run_id: &str,
    test_idx: usize,
    step_idx: usize,
    scope: &str,
    step: &StepRunResult,
    attachment_seq: &mut usize,
) -> Result<serde_json::Value, String> {
    let mut attachments: Vec<serde_json::Value> = Vec::new();

    if let Some(request) = &step.request {
        *attachment_seq += 1;
        let source = format!("{run_id}-{test_idx}-{step_idx}-{scope}-req-{attachment_seq}.json");
        let payload = serde_json::to_string_pretty(request)
            .map_err(|e| format!("failed to encode request attachment: {e}"))?;
        write_attachment_text(allure_dir, &source, &payload)?;
        attachments.push(json!({
            "name": "request",
            "source": source,
            "type": "application/json"
        }));
    }

    if let Some(response) = &step.response {
        *attachment_seq += 1;
        let source = format!("{run_id}-{test_idx}-{step_idx}-{scope}-resp-{attachment_seq}.json");
        let payload = serde_json::to_string_pretty(response)
            .map_err(|e| format!("failed to encode response attachment: {e}"))?;
        write_attachment_text(allure_dir, &source, &payload)?;
        attachments.push(json!({
            "name": "response",
            "source": source,
            "type": "application/json"
        }));
    }

    if !step.assertions.is_empty() {
        *attachment_seq += 1;
        let source = format!("{run_id}-{test_idx}-{step_idx}-{scope}-assert-{attachment_seq}.json");
        let payload = serde_json::to_string_pretty(&step.assertions)
            .map_err(|e| format!("failed to encode assertions attachment: {e}"))?;
        write_attachment_text(allure_dir, &source, &payload)?;
        attachments.push(json!({
            "name": "assertions",
            "source": source,
            "type": "application/json"
        }));
    }

    Ok(json!({
      "name": step.name,
      "status": step.status,
      "stage": "finished",
      "statusDetails": { "message": step.message },
      "attachments": attachments
    }))
}

pub fn write_allure_results(allure_dir: &Path, run_id: &str, results: &[TestRunResult]) -> Result<(), String> {
    fs::create_dir_all(allure_dir).map_err(|e| format!("failed to create {}: {e}", allure_dir.display()))?;
    for (idx, test) in results.iter().enumerate() {
        let test_idx = idx + 1;
        let mut attachment_seq = 0usize;
        let body_steps = test
            .steps
            .iter()
            .enumerate()
            .map(|(step_idx, step)| {
                encode_step_with_attachments(
                    allure_dir,
                    run_id,
                    test_idx,
                    step_idx + 1,
                    "body",
                    step,
                    &mut attachment_seq,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let setup_steps = test
            .setup_steps
            .iter()
            .enumerate()
            .map(|(step_idx, step)| {
                encode_step_with_attachments(
                    allure_dir,
                    run_id,
                    test_idx,
                    step_idx + 1,
                    "setup",
                    step,
                    &mut attachment_seq,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let teardown_steps = test
            .teardown_steps
            .iter()
            .enumerate()
            .map(|(step_idx, step)| {
                encode_step_with_attachments(
                    allure_dir,
                    run_id,
                    test_idx,
                    step_idx + 1,
                    "teardown",
                    step,
                    &mut attachment_seq,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let test_uuid = format!("{}-{}", run_id, test_idx);
        let payload = json!({
            "uuid": test_uuid,
            "historyId": format!("{}::{}", test.id, test.file),
            "name": test.title,
            "status": test.status,
            "stage": "finished",
            "labels": build_suite_labels(&test.file),
            "steps": body_steps
        });
        write_json(&allure_dir.join(format!("{}-result.json", test_idx)), &payload)?;

        let container_payload = json!({
            "uuid": format!("{}-{}-container", run_id, test_idx),
            "name": test.title,
            "children": [test_uuid],
            "befores": setup_steps,
            "afters": teardown_steps
        });
        write_json(
            &allure_dir.join(format!("{}-container.json", test_idx)),
            &container_payload,
        )?;
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

    let suites_root = discovered.root.join(manifest.suites_dir_or_default());
    let files = collect_selected_files(
        &discovered.root,
        &manifest.suites_dir_or_default(),
        options.test_path,
        options.suite_path,
    )?;
    let suite_init_cache = load_suite_init_cache(&suites_root)?;

    let started_at_ms = now_ms()?;
    let run_id = format!("run-{}", started_at_ms);
    let run_started = std::time::Instant::now();

    let mut results = Vec::new();
    let mut before_all_done = HashSet::new();
    let mut before_all_results: HashMap<String, HookExecutionResult> = HashMap::new();
    let mut suite_result_indexes: HashMap<String, Vec<usize>> = HashMap::new();
    for file in files {
        let content = fs::read_to_string(&file)
            .map_err(|e| format!("failed to read test file {}: {e}", file.display()))?;
        let parsed = parse_and_validate_test(&content, &file.to_string_lossy())?;

        if !matches_tag_filter(&parsed.tags, &options.tags) {
            continue;
        }

        let rel = relative_unix(&discovered.root, &file);
        let suite_ctx = build_suite_execution_context(&suites_root, &file, &suite_init_cache);
        let mut effective_vars = env_vars.clone();
        for (k, v) in &suite_ctx.suite_variables {
            effective_vars.insert(k.clone(), v.clone());
        }

        if !before_all_done.contains(&suite_ctx.suite_key) {
            before_all_done.insert(suite_ctx.suite_key.clone());
            if let Some(step) = run_hook_steps(
                "beforeAll",
                "setup",
                &suite_ctx.before_all,
                &file,
                &suite_ctx.suite_key,
                &base_url,
                &effective_vars,
            )
            .await
            {
                before_all_results.insert(suite_ctx.suite_key.clone(), step);
            }
        }

        if let Some(before_all_exec) = before_all_results.get(&suite_ctx.suite_key) {
            if before_all_exec.status == "failed" {
                results.push(TestRunResult {
                    id: parsed.id,
                    title: parsed.title,
                    tags: parsed.tags,
                    file: rel,
                    status: "failed".to_string(),
                    duration_ms: 0,
                    errors: vec![format!("setup failure in beforeAll: {}", before_all_exec.message)],
                    steps: Vec::new(),
                    setup_steps: before_all_exec.steps.clone(),
                    teardown_steps: Vec::new(),
                });
                let idx = results.len() - 1;
                suite_result_indexes
                    .entry(suite_ctx.suite_key.clone())
                    .or_default()
                    .push(idx);
                continue;
            }
        }

        let before_each_exec = run_hook_steps(
            "beforeEach",
            "setup",
            &suite_ctx.before_each,
            &file,
            &suite_ctx.suite_key,
            &base_url,
            &effective_vars,
        )
        .await;

        let mut test_result = if let Some(exec) = before_each_exec.clone() {
            if exec.status == "failed" {
                TestRunResult {
                    id: parsed.id,
                    title: parsed.title,
                    tags: parsed.tags,
                    file: rel,
                    status: "failed".to_string(),
                    duration_ms: 0,
                    errors: vec![format!("setup failure in beforeEach: {}", exec.message)],
                    steps: Vec::new(),
                    setup_steps: exec.steps,
                    teardown_steps: Vec::new(),
                }
            } else {
                run_test(&parsed, &file, rel, &base_url, &effective_vars).await
            }
        } else {
            run_test(&parsed, &file, rel, &base_url, &effective_vars).await
        };

        if let Some(exec) = before_each_exec {
            if exec.status != "failed" {
                test_result.setup_steps.extend(exec.steps);
            }
        }

        if let Some(before_all_exec) = before_all_results.get(&suite_ctx.suite_key) {
            let mut combined = before_all_exec.steps.clone();
            combined.extend(test_result.setup_steps.clone());
            test_result.setup_steps = combined;
        }

        if let Some(after_each_exec) = run_hook_steps(
            "afterEach",
            "teardown",
            &suite_ctx.after_each,
            &file,
            &suite_ctx.suite_key,
            &base_url,
            &effective_vars,
        )
        .await
        {
            if after_each_exec.status == "failed" {
                test_result.status = "failed".to_string();
                test_result
                    .errors
                    .push(format!("teardown failure in afterEach: {}", after_each_exec.message));
            }
            test_result.teardown_steps.extend(after_each_exec.steps);
        }

        results.push(test_result);
        let idx = results.len() - 1;
        suite_result_indexes
            .entry(suite_ctx.suite_key.clone())
            .or_default()
            .push(idx);
    }

    // Run afterAll once per suite and attach teardown status to all tests in that suite.
    let mut all_suites: Vec<String> = suite_result_indexes.keys().cloned().collect();
    all_suites.sort_by_key(|k| std::cmp::Reverse(k.matches('/').count()));
    for suite_key in all_suites {
        let representative = results
            .iter()
            .find(|r| {
                let p = discovered.root.join(&r.file);
                suite_key_for_test(&suites_root, &p) == suite_key
            })
            .map(|r| discovered.root.join(&r.file));
        let Some(test_path) = representative else {
            continue;
        };
        let suite_ctx = build_suite_execution_context(&suites_root, &test_path, &suite_init_cache);
        let mut effective_vars = env_vars.clone();
        for (k, v) in &suite_ctx.suite_variables {
            effective_vars.insert(k.clone(), v.clone());
        }
        if let Some(after_all_exec) = run_hook_steps(
            "afterAll",
            "teardown",
            &suite_ctx.after_all,
            &test_path,
            &suite_ctx.suite_key,
            &base_url,
            &effective_vars,
        )
        .await
        {
            if let Some(indexes) = suite_result_indexes.get(&suite_key) {
                for idx in indexes {
                    if let Some(result) = results.get_mut(*idx) {
                        if after_all_exec.status == "failed" {
                            result.status = "failed".to_string();
                            result
                                .errors
                                .push(format!("teardown failure in afterAll: {}", after_all_exec.message));
                        }
                        result.teardown_steps.extend(after_all_exec.steps.clone());
                    }
                }
            }
        }
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

