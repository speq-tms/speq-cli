use crate::cli::discovery::discover_speq_root;
use crate::cli::files::{collect_yaml_files, relative_unix};
use crate::manifest::read_manifest;
use crate::parser::parse_and_validate_test;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListedTest {
    id: String,
    title: String,
    file: String,
}

pub fn command_list(speq_root_override: Option<String>, format_json: bool) -> Result<(), String> {
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

    let mut tests = Vec::new();
    for file in &files {
        let content = std::fs::read_to_string(file)
            .map_err(|e| format!("failed to read suite {}: {e}", file.display()))?;
        let file_label = file.to_string_lossy().to_string();
        let parsed = parse_and_validate_test(&content, &file_label)?;
        tests.push(ListedTest {
            id: parsed.id,
            title: parsed.title,
            file: relative_unix(&discovered.root, file),
        });
    }

    if format_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&tests)
                .map_err(|e| format!("internal: failed to encode json: {e}"))?
        );
    } else {
        for item in tests {
            println!("{}  {}  {}", item.id, item.title, item.file);
        }
    }

    Ok(())
}
