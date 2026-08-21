use std::env;
use std::ffi::OsStr;
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

    // Walk from the current directory towards the filesystem root and take the
    // first directory that looks like a project, the way git, cargo and npm
    // resolve theirs. The nearest root wins; a candidate further up is never
    // considered once a nearer one matches.
    for dir in cwd.ancestors() {
        let in_repo = is_in_repo_mode(dir);
        let test_repo = is_test_repo_mode(dir);

        // Two layouts in one directory is still a refusal to guess. Name the
        // directory, because with the walk it need not be the one you are in.
        if in_repo && test_repo {
            return Err(format!(
                "ambiguous speq layout in {}: both .speq and repository root look valid, pass --speq-root",
                dir.display()
            ));
        }
        if in_repo {
            return Ok(SpeqRoot {
                mode: "in-repo".to_string(),
                root: dir.join(".speq"),
            });
        }
        if test_repo {
            // Standing inside an in-repo project's own .speq directory reaches
            // the same root from below. Reporting that as test-repo would
            // misdescribe the layout to `doctor` and `validate`.
            let mode = if dir.file_name() == Some(OsStr::new(".speq")) {
                "in-repo"
            } else {
                "test-repo"
            };
            return Ok(SpeqRoot {
                mode: mode.to_string(),
                root: dir.to_path_buf(),
            });
        }
    }

    Err(format!(
        "speq root not found in {} or any parent directory; run 'speq init' or pass --speq-root",
        cwd.display()
    ))
}
