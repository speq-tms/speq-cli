use std::process::Command;

fn speq(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_speq"))
        .args(args)
        .output()
        .expect("run speq binary")
}

#[test]
fn help_command_prints_available_commands() {
    let output = speq(&["help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert!(stdout.contains("speq CLI"));
    assert!(stdout.contains("speq run"));
    assert!(stdout.contains("speq version"));
    assert!(stdout.contains("speq help"));
}

#[test]
fn version_command_prints_package_version() {
    let output = speq(&["version"]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf8");
    assert_eq!(stdout.trim(), format!("speq {}", env!("CARGO_PKG_VERSION")));
}
