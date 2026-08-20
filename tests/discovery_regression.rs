use speq_cli::cli::discovery::discover_speq_root;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_tmp_dir(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("speq-cli-test-{}-{}", name, suffix));
    fs::create_dir_all(&path).expect("create temp dir");
    // The macOS temp dir is a symlink, and the child process reports the
    // resolved path, so compare against the resolved one.
    path.canonicalize().expect("canonicalize temp dir")
}

/// Write the pair that marks a speq root: a manifest and a suites directory.
fn make_project(root: &Path, project: &str) {
    fs::create_dir_all(root.join("suites")).expect("create suites");
    fs::create_dir_all(root.join("environments")).expect("create environments");
    fs::write(
        root.join("manifest.yaml"),
        format!("version: \"1\"\nproject: \"{project}\"\ndefaultEnvironment: \"ci\"\n"),
    )
    .expect("write manifest");
}

/// Run `speq doctor --format json` with the given working directory.
fn doctor_in(cwd: &Path, extra: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_speq"))
        .args(["doctor", "--format", "json"])
        .args(extra)
        .current_dir(cwd)
        .output()
        .expect("run speq binary");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("stdout is utf8"),
        String::from_utf8(output.stderr).expect("stderr is utf8"),
    )
}

fn field<'a>(json: &'a str, key: &str) -> String {
    let parsed: serde_json::Value = serde_json::from_str(json).expect("doctor emits json");
    parsed[key].as_str().expect("string field").to_string()
}

#[test]
fn discovery_accepts_explicit_path() {
    let result = discover_speq_root(Some("./src".to_string()));
    assert!(result.is_ok());
}

#[test]
fn finds_a_test_repo_root_from_a_nested_subdirectory() {
    let root = make_tmp_dir("discover-nested");
    make_project(&root, "nested");
    let deep = root.join("nested").join("deeper");
    fs::create_dir_all(&deep).expect("create nested dirs");

    let (ok, stdout, stderr) = doctor_in(&deep, &[]);
    assert!(ok, "doctor failed: {stderr}");
    assert_eq!(field(&stdout, "mode"), "test-repo");
    assert_eq!(field(&stdout, "speqRoot"), root.to_string_lossy());
}

#[test]
fn finds_an_in_repo_root_from_a_sibling_subdirectory() {
    let repo = make_tmp_dir("discover-in-repo");
    make_project(&repo.join(".speq"), "in-repo");
    let src = repo.join("src").join("deep");
    fs::create_dir_all(&src).expect("create src dirs");

    let (ok, stdout, stderr) = doctor_in(&src, &[]);
    assert!(ok, "doctor failed: {stderr}");
    assert_eq!(field(&stdout, "mode"), "in-repo");
    assert_eq!(field(&stdout, "speqRoot"), repo.join(".speq").to_string_lossy());
}

#[test]
fn the_nearest_root_wins() {
    let outer = make_tmp_dir("discover-nearest");
    make_project(&outer, "outer");
    let inner = outer.join("packages").join("inner");
    make_project(&inner, "inner");
    let deep = inner.join("suites").join("group");
    fs::create_dir_all(&deep).expect("create inner suites");

    let (ok, stdout, stderr) = doctor_in(&deep, &[]);
    assert!(ok, "doctor failed: {stderr}");
    assert_eq!(field(&stdout, "speqRoot"), inner.to_string_lossy());
}

#[test]
fn standing_inside_dot_speq_still_reports_in_repo() {
    let repo = make_tmp_dir("discover-inside-dot-speq");
    make_project(&repo.join(".speq"), "in-repo");
    let inside = repo.join(".speq").join("suites");

    let (ok, stdout, stderr) = doctor_in(&inside, &[]);
    assert!(ok, "doctor failed: {stderr}");
    assert_eq!(field(&stdout, "mode"), "in-repo");
    assert_eq!(field(&stdout, "speqRoot"), repo.join(".speq").to_string_lossy());
}

#[test]
fn same_directory_ambiguity_is_still_an_error() {
    let root = make_tmp_dir("discover-ambiguous");
    make_project(&root, "both");
    make_project(&root.join(".speq"), "both-dot-speq");
    let deep = root.join("nested");
    fs::create_dir_all(&deep).expect("create nested dir");

    let (ok, _stdout, stderr) = doctor_in(&deep, &[]);
    assert!(!ok, "ambiguous layout should fail");
    assert!(
        stderr.contains("ambiguous speq layout"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains(&root.to_string_lossy().to_string()),
        "error should name the ambiguous directory, got: {stderr}"
    );
}

#[test]
fn a_nearer_root_wins_over_an_ambiguous_ancestor() {
    let outer = make_tmp_dir("discover-ambiguous-ancestor");
    make_project(&outer, "outer");
    make_project(&outer.join(".speq"), "outer-dot-speq");
    let inner = outer.join("packages").join("inner");
    make_project(&inner, "inner");

    let (ok, stdout, stderr) = doctor_in(&inner, &[]);
    assert!(ok, "the nearer root should be used, stderr: {stderr}");
    assert_eq!(field(&stdout, "speqRoot"), inner.to_string_lossy());
}

#[test]
fn not_found_says_the_parents_were_searched() {
    let empty = make_tmp_dir("discover-none");
    let deep = empty.join("a").join("b");
    fs::create_dir_all(&deep).expect("create dirs");

    let (ok, _stdout, stderr) = doctor_in(&deep, &[]);
    assert!(!ok, "an empty tree should not resolve");
    assert!(
        stderr.contains("or any parent directory"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn speq_root_flag_short_circuits_the_search() {
    // A project the walk would find, and a different one named explicitly.
    let root = make_tmp_dir("discover-override");
    make_project(&root, "walked");
    let elsewhere = make_tmp_dir("discover-override-target");
    make_project(&elsewhere, "explicit");
    let deep = root.join("nested");
    fs::create_dir_all(&deep).expect("create nested dir");

    let target = elsewhere.to_string_lossy().to_string();
    let (ok, stdout, stderr) = doctor_in(&deep, &["--speq-root", &target]);
    assert!(ok, "doctor failed: {stderr}");
    assert_eq!(field(&stdout, "mode"), "explicit");
    assert_eq!(field(&stdout, "speqRoot"), target);
}
