use speq_cli::cli::discovery::discover_speq_root;

#[test]
fn discovery_accepts_explicit_path() {
    let result = discover_speq_root(Some("./src".to_string()));
    assert!(result.is_ok());
}
