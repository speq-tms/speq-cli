use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn collect_yaml_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_file() {
            let is_yaml = path
                .extension()
                .and_then(|x| x.to_str())
                .map(|ext| {
                    let lower = ext.to_ascii_lowercase();
                    lower == "yaml" || lower == "yml"
                })
                .unwrap_or(false);
            if is_yaml {
                files.push(path.to_path_buf());
            }
        }
    }
    files.sort();
    files
}

pub fn relative_unix(base: &Path, target: &Path) -> String {
    target
        .strip_prefix(base)
        .unwrap_or(target)
        .to_string_lossy()
        .replace('\\', "/")
}
