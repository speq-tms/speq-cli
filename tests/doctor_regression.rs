use speq_cli::cli::doctor::command_doctor;

#[test]
fn doctor_works_on_canonical_example() {
    let root = format!(
        "{}/../speq-examples/test-repo-mode",
        env!("CARGO_MANIFEST_DIR")
    );
    let result = command_doctor(Some(root), true);
    assert!(result.is_ok());
}
