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

#[test]
fn assert_value_is_read_as_expected() {
    let content = r#"
id: "posts.assert.value"
title: "assert written with value"
steps:
  - type: api
    name: "GET post"
    method: GET
    url: "/posts/1"
    assert:
      - type: json
        path: "$.id"
        value: 999
      - type: status
        value: 200
"#;

    let parsed = parse_and_validate_test(content, "suites/posts/assert-value.yaml")
        .expect("value should parse as expected");
    let asserts = &parsed.steps[0].assertions;
    assert_eq!(asserts[0].expected, Some(serde_json::json!(999)));
    assert_eq!(asserts[1].expected, Some(serde_json::json!(200)));
}

#[test]
fn assert_rejects_expected_and_value_together() {
    let content = r#"
id: "posts.assert.both"
title: "assert written with both spellings"
steps:
  - type: api
    name: "GET post"
    method: GET
    url: "/posts/1"
    assert:
      - type: json
        path: "$.id"
        expected: 1
        value: 999
"#;

    let err = parse_and_validate_test(content, "suites/posts/assert-both.yaml")
        .expect_err("expected and value in one assertion should fail");
    assert!(
        err.contains("expected"),
        "error should name the field, got: {err}"
    );
}
