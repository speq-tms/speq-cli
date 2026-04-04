use crate::cli::discovery::discover_speq_root;
use crate::cli::files::collect_yaml_files;
use crate::manifest::read_manifest;
use serde_json::json;

pub fn command_doctor(speq_root_override: Option<String>, format_json: bool) -> Result<(), String> {
    let discovered = discover_speq_root(speq_root_override)?;
    let manifest_path = discovered.root.join("manifest.yaml");
    let manifest_exists = manifest_path.is_file();

    let manifest = read_manifest(&discovered.root)?;
    let suites_dir = discovered.root.join(manifest.suites_dir_or_default());
    let environments_dir = discovered
        .root
        .join(manifest.environments_dir.clone().unwrap_or_else(|| "environments".to_string()));
    let reports_dir = discovered
        .root
        .join(manifest.reports_dir.clone().unwrap_or_else(|| "reports".to_string()));

    let tests_count = if suites_dir.is_dir() {
        collect_yaml_files(&suites_dir).len()
    } else {
        0
    };

    let payload = json!({
      "ok": manifest_exists && suites_dir.is_dir() && environments_dir.is_dir(),
      "mode": discovered.mode,
      "speqRoot": discovered.root.to_string_lossy(),
      "manifestExists": manifest_exists,
      "suitesDir": suites_dir.to_string_lossy(),
      "suitesDirExists": suites_dir.is_dir(),
      "environmentsDir": environments_dir.to_string_lossy(),
      "environmentsDirExists": environments_dir.is_dir(),
      "reportsDir": reports_dir.to_string_lossy(),
      "testsCount": tests_count
    });

    if format_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .map_err(|e| format!("internal: failed to encode doctor json: {e}"))?
        );
    } else if payload["ok"].as_bool().unwrap_or(false) {
        println!(
            "Doctor OK: mode={}, tests={}, root={}",
            payload["mode"],
            tests_count,
            discovered.root.display()
        );
    } else {
        println!(
            "Doctor WARN: check failed, root={}, manifest={}, suites={}, envs={}",
            discovered.root.display(),
            manifest_exists,
            suites_dir.is_dir(),
            environments_dir.is_dir()
        );
    }

    Ok(())
}
