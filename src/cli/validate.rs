use crate::cli::discovery::discover_speq_root;
use crate::cli::files::collect_yaml_files;
use crate::manifest::read_manifest;
use crate::parser::parse_and_validate_test;
use serde_json::json;
use std::fs;

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

    let files = collect_yaml_files(&suites_dir);

    if files.is_empty() {
        return Err(format!("no suite YAML files found in {}", suites_dir.display()));
    }

    let mut errors = Vec::new();
    for file in &files {
        let content =
            fs::read_to_string(file).map_err(|e| format!("failed to read suite {}: {e}", file.display()))?;
        let file_label = file.to_string_lossy().to_string();
        if let Err(err) = parse_and_validate_test(&content, &file_label) {
            errors.push(err);
        }
    }

    if format_json {
        let payload = json!({
            "ok": errors.is_empty(),
            "mode": discovered.mode,
            "speqRoot": discovered.root.to_string_lossy(),
            "manifestVersion": manifest.version,
            "testsCount": files.len(),
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
