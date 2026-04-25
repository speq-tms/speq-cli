use speq_cli::parser::parse_and_validate_suite_init;

#[test]
fn init_yaml_accepts_suite_hooks() {
    let content = r#"
suite:
  variables:
    tenantId: "acme"
  imports:
    - module: auth
      alias: auth
  beforeAll:
    - type: api
      name: "prepare"
      method: POST
      url: "/setup"
  beforeEach:
    - type: api
      name: "login"
      method: POST
      url: "/login"
  afterEach:
    - type: api
      name: "logout"
      method: POST
      url: "/logout"
  afterAll:
    - type: api
      name: "cleanup"
      method: DELETE
      url: "/cleanup"
"#;
    let parsed = parse_and_validate_suite_init(content, "suites/api/init.yaml").expect("parse init");
    assert_eq!(parsed.suite.variables.get("tenantId").and_then(|v| v.as_str()), Some("acme"));
    assert_eq!(parsed.suite.imports.len(), 1);
    assert_eq!(parsed.suite.imports[0].module, "auth");
    assert_eq!(parsed.suite.before_all.len(), 1);
    assert_eq!(parsed.suite.before_each.len(), 1);
    assert_eq!(parsed.suite.after_each.len(), 1);
    assert_eq!(parsed.suite.after_all.len(), 1);
}
