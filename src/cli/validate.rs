use crate::cli::discovery::discover_speq_root;
use crate::cli::files::{collect_suite_init_files, collect_yaml_files};
use crate::fixtures::load_fixture;
use crate::manifest::read_manifest;
use crate::parser::{parse_and_validate_suite_init, parse_and_validate_test};
use crate::runner::validate_module_content;
use serde_json::json;
use std::fs;
use std::path::Path;

fn validate_module_files(modules_root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    if !modules_root.is_dir() {
        return errors;
    }
    for file in collect_yaml_files(modules_root) {
        match fs::read_to_string(&file) {
            Ok(content) => {
                let label = file.to_string_lossy().to_string();
                errors.extend(validate_module_content(&content, &label));
            }
            Err(e) => errors.push(format!("failed to read module {}: {e}", file.display())),
        }
    }
    errors
}

fn validate_fixture_refs(
    parsed: &crate::parser::TestSpec,
    fixtures_root: &Path,
    schemas_root: &Path,
    file_label: &str,
) -> Vec<String> {
    let mut errors = Vec::new();
    let all_steps = parsed
        .setup
        .iter()
        .chain(parsed.steps.iter())
        .chain(parsed.cleanup.iter());
    for step in all_steps {
        if let Some(bff) = &step.body_from_fixture {
            let fixture_path = fixtures_root.join(&bff.r#ref);
            match load_fixture(&fixture_path) {
                Ok(cfg) => {
                    if let Some(schema_ref) = &cfg.schema_ref {
                        let schema_path = schemas_root.join(schema_ref);
                        if !schema_path.exists() {
                            errors.push(format!(
                                "fixture_resolution_error: schemaRef '{}' not found (referenced in {} step '{}')",
                                schema_ref, file_label, step.name
                            ));
                        }
                    }
                }
                Err(e) => {
                    errors.push(format!("{} (referenced in {} step '{}')", e, file_label, step.name));
                }
            }
        }
    }
    errors
}

pub fn command_validate(speq_root_override: Option<String>, format_json: bool) -> Result<(), String> {
    let discovered = discover_speq_root(speq_root_override)?;
    let manifest = read_manifest(&discovered.root)?;

    let suites_dir = discovered.root.join(manifest.suites_dir_or_default());
    if !suites_dir.is_dir() {
        return Err(format!(
            "suites directory does not exist: {}",
            suites_dir.display()
        ));
    }

    let fixtures_root = discovered.root.join(manifest.fixtures_dir_or_default());
    let schemas_root = discovered.root.join(manifest.schemas_dir_or_default());
    let modules_root = discovered.root.join(manifest.modules_dir_or_default());

    let files = collect_yaml_files(&suites_dir);
    let init_files = collect_suite_init_files(&suites_dir);

    if files.is_empty() {
        return Err(format!("no suite YAML files found in {}", suites_dir.display()));
    }

    let mut errors = Vec::new();
    for file in &files {
        let content =
            fs::read_to_string(file).map_err(|e| format!("failed to read suite {}: {e}", file.display()))?;
        let file_label = file.to_string_lossy().to_string();
        match parse_and_validate_test(&content, &file_label) {
            Ok(parsed) => {
                let fixture_errors = validate_fixture_refs(&parsed, &fixtures_root, &schemas_root, &file_label);
                errors.extend(fixture_errors);
            }
            Err(err) => {
                errors.push(err);
            }
        }
    }

    for file in &init_files {
        let content =
            fs::read_to_string(file).map_err(|e| format!("failed to read suite init {}: {e}", file.display()))?;
        let file_label = file.to_string_lossy().to_string();
        if let Err(err) = parse_and_validate_suite_init(&content, &file_label) {
            errors.push(err);
        }
    }

    errors.extend(validate_module_files(&modules_root));

    if format_json {
        let payload = json!({
            "ok": errors.is_empty(),
            "mode": discovered.mode,
            "speqRoot": discovered.root.to_string_lossy(),
            "manifestVersion": manifest.version,
            "testsCount": files.len(),
            "suiteInitCount": init_files.len(),
            "errors": errors
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| format!("internal: failed to encode json: {e}"))?
        );
    } else if errors.is_empty() {
        println!(
            "Validation passed: mode={}, root={}, tests={}",
            discovered.mode,
            discovered.root.display(),
            files.len()
        );
    } else {
        println!("Validation failed:");
        for err in &errors {
            println!("- {err}");
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err("validation failed".to_string())
    }
}
