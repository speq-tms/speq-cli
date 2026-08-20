use speq_cli::manifest::read_manifest;
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
fn manifest_supports_default_schema_and_modules_dirs() {
    let root = make_tmp_dir("manifest-defaults");
    fs::write(
        root.join("manifest.yaml"),
        r#"
version: "1"
project: "demo"
defaultEnvironment: "ci"
"#,
    )
    .expect("write manifest");

    let manifest = read_manifest(&root).expect("manifest should parse");
    assert_eq!(manifest.schemas_dir_or_default(), "schemas");
    assert_eq!(manifest.modules_dir_or_default(), "modules");
}

#[test]
fn manifest_reads_custom_schema_and_modules_dirs() {
    let root = make_tmp_dir("manifest-custom");
    fs::write(
        root.join("manifest.yaml"),
        r#"
version: "1"
project: "demo"
defaultEnvironment: "ci"
schemasDir: "contracts/schemas"
modulesDir: "shared/modules"
"#,
    )
    .expect("write manifest");

    let manifest = read_manifest(&root).expect("manifest should parse");
    assert_eq!(manifest.schemas_dir_or_default(), "contracts/schemas");
    assert_eq!(manifest.modules_dir_or_default(), "shared/modules");
}

#[test]
fn coverage_threshold_reads_camel_case() {
    let root = make_tmp_dir("manifest-fail-below-camel");
    fs::write(
        root.join("manifest.yaml"),
        r#"
version: "1"
project: "demo"
defaultEnvironment: "ci"
coverage:
  enabled: true
  openapi: "openapi.yaml"
  failBelow: 80
"#,
    )
    .expect("write manifest");

    let manifest = read_manifest(&root).expect("manifest should parse");
    let coverage = manifest.coverage.expect("coverage block");
    assert_eq!(coverage.fail_below, Some(80.0));
}

#[test]
fn coverage_threshold_still_reads_the_snake_case_alias() {
    let root = make_tmp_dir("manifest-fail-below-snake");
    fs::write(
        root.join("manifest.yaml"),
        r#"
version: "1"
project: "demo"
defaultEnvironment: "ci"
coverage:
  enabled: true
  fail_below: 42.5
"#,
    )
    .expect("write manifest");

    let manifest = read_manifest(&root).expect("manifest should parse");
    let coverage = manifest.coverage.expect("coverage block");
    assert_eq!(coverage.fail_below, Some(42.5));
}
