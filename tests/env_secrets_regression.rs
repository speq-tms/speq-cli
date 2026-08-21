//! End-to-end cover for `${ENV_VAR}` (speq-tms/speq-docs#60).
//!
//! The unit tests in `src/secrets` cover the syntax. What they cannot cover is
//! the promise that matters: a value sourced from the environment does not
//! appear in anything the run writes down. That has to be checked against the
//! real binary and the real report files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SECRET: &str = "sp3q-t0ken-do-not-print-me";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "speq-env-secrets-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A project whose environment reads its token from the OS, plus a test that
/// sends it as a header to a local server.
fn write_project(root: &Path, base_url: &str) {
    fs::write(
        root.join("manifest.yaml"),
        "version: \"1\"\nproject: env-secrets\ndefaultEnvironment: ci\n",
    )
    .expect("manifest");

    fs::create_dir_all(root.join("environments")).expect("env dir");
    fs::write(
        root.join("environments/ci.yaml"),
        format!(
            "name: ci\nbaseUrl: {base_url}\nheaders:\n  authorization: 'Bearer ${{SPEQ_IT_TOKEN}}'\n\
             region: '${{SPEQ_IT_ABSENT:-eu-west-1}}'\n"
        ),
    )
    .expect("env file");

    fs::create_dir_all(root.join("suites")).expect("suites dir");
    fs::write(
        root.join("suites/probe.yaml"),
        "id: env.secrets.probe\ntitle: sends a secret header\nsteps:\n  \
         - type: api\n    name: probe\n    method: GET\n    url: /probe\n    \
         assert:\n      - type: status\n        expected: 200\n",
    )
    .expect("test file");
}

fn speq(root: &Path, args: &[&str], token: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_speq"));
    cmd.args(args).arg("--speq-root").arg(root);
    match token {
        Some(v) => cmd.env("SPEQ_IT_TOKEN", v),
        None => cmd.env_remove("SPEQ_IT_TOKEN"),
    };
    cmd.env_remove("SPEQ_IT_ABSENT");
    cmd.output().expect("run speq binary")
}

/// Answers 200 to anything, echoing nothing. Enough for the run to produce a
/// full set of report files.
fn spawn_server() -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = std::thread::spawn(move || {
        for stream in listener.incoming().take(1) {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = "{\"ok\":true}";
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    (format!("http://127.0.0.1:{port}"), handle)
}

#[test]
fn a_resolved_secret_never_reaches_the_reports() {
    let (base_url, server) = spawn_server();
    let root = scratch("redaction");
    write_project(&root, &base_url);

    let output = speq(&root, &["run", "--env", "ci", "--report", "all"], Some(SECRET));
    let _ = server.join();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "run should pass; stdout={stdout} stderr={stderr}"
    );

    // Every file the run wrote, plus everything it printed.
    let mut written = vec![stdout, stderr];
    for dir in ["reports/results", "reports/allure"] {
        let path = root.join(dir);
        if !path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&path).expect("read reports dir") {
            let entry = entry.expect("dir entry");
            if entry.path().is_file() {
                written.push(fs::read_to_string(entry.path()).unwrap_or_default());
            }
        }
    }
    assert!(
        written.len() > 2,
        "expected the run to write report files, found none under {}",
        root.display()
    );

    let authorization_seen = written.iter().any(|text| text.contains("authorization"));
    assert!(
        authorization_seen,
        "the header must reach the request attachment, or this test proves nothing"
    );
    for text in &written {
        assert!(
            !text.contains(SECRET),
            "a value sourced from ${{ENV_VAR}} leaked into run output:\n{text}"
        );
        }
    assert!(
        written.iter().any(|t| t.contains("***")),
        "the secret should be present but redacted, not simply absent"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn validate_reports_an_unresolvable_variable_without_running_anything() {
    let root = scratch("validate");
    write_project(&root, "http://127.0.0.1:1");

    let output = speq(&root, &["validate"], None);

    assert!(!output.status.success(), "validate must fail on an unset variable");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("unresolved_env_var") && combined.contains("SPEQ_IT_TOKEN"),
        "the error must name the variable: {combined}"
    );
    assert!(
        combined.contains("ci.yaml"),
        "the error must name the file that referenced it: {combined}"
    );
    assert!(
        !root.join("reports").exists(),
        "validate must not execute anything"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn a_default_keeps_a_project_runnable_without_the_variable() {
    let root = scratch("default");
    write_project(&root, "http://127.0.0.1:1");
    // Only the token is missing a default, so supplying it leaves the region
    // placeholder to fall back on its own.
    let output = speq(&root, &["validate"], Some(SECRET));

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "validate should pass once the variable resolves: {combined}"
    );

    let _ = fs::remove_dir_all(&root);
}
