use speq_cli::parser::parse_and_validate_test;

#[test]
fn schema_assert_accepts_ref() {
    let content = r#"
id: "users.schema"
title: "Schema assertion with ref"
steps:
  - type: api
    name: "GET users"
    method: GET
    url: "/users/1"
    assert:
      - type: schema
        ref: "user.json"
"#;

    let parsed = parse_and_validate_test(content, "suites/users/schema.yaml").expect("schema ref should pass");
    assert_eq!(parsed.steps.len(), 1);
}

#[test]
fn schema_assert_accepts_inline() {
    let content = r#"
id: "users.schema.inline"
title: "Schema assertion with inline schema"
steps:
  - type: api
    name: "GET users"
    method: GET
    url: "/users/1"
    assert:
      - type: schema
        inline:
          type: object
"#;

    let parsed =
        parse_and_validate_test(content, "suites/users/schema-inline.yaml").expect("schema inline should pass");
    assert_eq!(parsed.steps.len(), 1);
}

#[test]
fn schema_assert_requires_ref_or_inline() {
    let content = r#"
id: "users.schema.invalid"
title: "Schema assertion missing target"
steps:
  - type: api
    name: "GET users"
    method: GET
    url: "/users/1"
    assert:
      - type: schema
"#;

    let err = parse_and_validate_test(content, "suites/users/schema-invalid.yaml")
        .expect_err("schema assert without ref or inline must fail");
    assert!(err.contains("requires 'ref' or 'inline'"));
}

#[test]
fn use_step_accepts_action_without_ref() {
    let content = r#"
id: "users.use.action"
title: "Use action"
steps:
  - type: use
    name: "Login action"
    action: "auth.login"
"#;

    let parsed = parse_and_validate_test(content, "suites/users/use-action.yaml").expect("use action should pass");
    assert_eq!(
        parsed.steps.first().and_then(|step| step.action.as_deref()),
        Some("auth.login")
    );
}

#[test]
fn use_step_rejects_missing_action_and_ref() {
    let content = r#"
id: "users.use.invalid"
title: "Use without target"
steps:
  - type: use
    name: "Broken use"
"#;

    let err = parse_and_validate_test(content, "suites/users/use-invalid.yaml")
        .expect_err("use step without action/ref must fail");
    assert!(err.contains("action or ref is required"));
}

#[test]
fn imports_are_parsed_and_validated() {
    let content = r#"
id: "users.imports"
title: "Imports support"
imports:
  - module: auth
    alias: auth
  - module: common/helpers
steps:
  - type: use
    name: "Login action"
    action: "auth.login"
"#;

    let parsed = parse_and_validate_test(content, "suites/users/imports.yaml").expect("imports should parse");
    assert_eq!(parsed.imports.len(), 2);
    assert_eq!(parsed.imports[0].module, "auth");
    assert_eq!(parsed.imports[0].alias.as_deref(), Some("auth"));
}

#[test]
fn use_step_accepts_properties_payload() {
    let content = r#"
id: "users.use.props"
title: "Use with properties"
steps:
  - type: use
    name: "Get post by id"
    action: "posts.getById"
    properties:
      postId: 42
"#;

    let parsed = parse_and_validate_test(content, "suites/users/use-props.yaml").expect("use properties should parse");
    assert_eq!(
        parsed.steps[0]
            .properties
            .get("postId")
            .and_then(|v| v.as_i64()),
        Some(42)
    );
}
