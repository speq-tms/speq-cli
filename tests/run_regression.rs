use speq_cli::cli::run::{
    build_run_options, collect_selected_files, matches_tag_filter, parse_tags_csv, resolve_report_mode, ReportMode,
};
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
fn tags_filter_matches_any() {
    let test_tags = vec!["smoke".to_string(), "api".to_string()];
    let filter_tags = vec!["regression".to_string(), "api".to_string()];
    assert!(matches_tag_filter(&test_tags, &filter_tags));
    assert!(!matches_tag_filter(
        &test_tags,
        &["ui".to_string(), "slow".to_string()]
    ));
}

#[test]
fn parse_tags_csv_splits_and_trims() {
    let tags = parse_tags_csv(" smoke, api , ,nightly ");
    assert_eq!(tags, vec!["smoke", "api", "nightly"]);
}

#[test]
fn report_mode_defaults_to_allure() {
    let mode = resolve_report_mode(None).expect("default mode");
    assert!(matches!(mode, ReportMode::Allure));
}

#[test]
fn build_run_options_keeps_output_flag() {
    let opts = build_run_options(
        None,
        Some("ci".to_string()),
        None,
        Some("suites".to_string()),
        Some("smoke,api".to_string()),
        Some("summary".to_string()),
        Some("reports/results/custom.json".to_string()),
    )
    .expect("options");
    assert_eq!(opts.tags, vec!["smoke".to_string(), "api".to_string()]);
    assert_eq!(
        opts.summary_output,
        Some("reports/results/custom.json".to_string())
    );
}

#[test]
fn selected_files_rejects_test_and_suite_together() {
    let tmp = make_tmp_dir("collect-files-conflict");
    let result = collect_selected_files(
        &tmp,
        "suites",
        Some("a.yaml".to_string()),
        Some("suites".to_string()),
    );
    assert!(result.is_err());
}

#[test]
fn selected_files_works_for_suite_directory() {
    let tmp = make_tmp_dir("collect-files-suite");
    let suites = tmp.join("suites");
    fs::create_dir_all(&suites).expect("create suites");
    fs::write(suites.join("a.yaml"), "id: t1\ntitle: t1\nsteps: []\n").expect("write");
    fs::write(suites.join("b.yml"), "id: t2\ntitle: t2\nsteps: []\n").expect("write");
    fs::write(suites.join("ignore.txt"), "x").expect("write");

    let files = collect_selected_files(&tmp, "suites", None, Some("suites".to_string())).expect("files");
    assert_eq!(files.len(), 2);
}
