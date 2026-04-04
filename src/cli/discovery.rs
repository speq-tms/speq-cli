use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SpeqRoot {
    pub mode: String,
    pub root: PathBuf,
}

fn is_in_repo_mode(repo_root: &Path) -> bool {
    let dot_speq = repo_root.join(".speq");
    dot_speq.join("manifest.yaml").is_file() && dot_speq.join("suites").is_dir()
}

fn is_test_repo_mode(repo_root: &Path) -> bool {
    repo_root.join("manifest.yaml").is_file() && repo_root.join("suites").is_dir()
}

pub fn discover_speq_root(override_path: Option<String>) -> Result<SpeqRoot, String> {
    let cwd = env::current_dir().map_err(|e| format!("internal: failed to read current directory: {e}"))?;

    if let Some(raw) = override_path {
        let resolved = if Path::new(&raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            cwd.join(raw)
        };
        return Ok(SpeqRoot {
            mode: "explicit".to_string(),
            root: resolved,
        });
    }

    let in_repo = is_in_repo_mode(&cwd);
    let test_repo = is_test_repo_mode(&cwd);

    if in_repo && test_repo {
        return Err(
            "ambiguous speq layout: both .speq and repository root look valid, pass --speq-root".to_string(),
        );
    }
    if in_repo {
        return Ok(SpeqRoot {
            mode: "in-repo".to_string(),
            root: cwd.join(".speq"),
        });
    }
    if test_repo {
        return Ok(SpeqRoot {
            mode: "test-repo".to_string(),
            root: cwd,
        });
    }
    Err("speq root not found; run 'speq init' or pass --speq-root".to_string())
}
