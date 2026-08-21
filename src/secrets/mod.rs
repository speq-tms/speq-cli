//! `${ENV_VAR}` substitution and the redaction that has to follow it.
//!
//! Values are resolved from the process environment while a file is loaded,
//! before anything is parsed into a typed struct. Every value that came from
//! the environment is recorded, because the whole point of sourcing a token
//! from the environment is that it must not appear in a report afterwards.
//!
//! Loading a `.env` file is deliberately not supported — the OS environment is
//! the only source.

use std::sync::{LazyLock, RwLock};

/// What a secret is replaced with wherever it would otherwise be printed.
pub const REDACTED: &str = "***";

/// The set of values this process resolved from the environment.
///
/// Sorted longest first so that a secret which contains another secret is
/// replaced as a whole rather than leaving a fragment behind.
#[derive(Debug, Default, Clone)]
pub struct SecretRegistry {
    values: Vec<String>,
}

impl SecretRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Empty values are ignored: replacing an empty string would match at every
    /// position and redact nothing meaningful.
    pub fn record(&mut self, value: &str) {
        if value.is_empty() || self.values.iter().any(|v| v == value) {
            return;
        }
        self.values.push(value.to_string());
        self.values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for value in &self.values {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), REDACTED);
            }
        }
        out
    }
}

static GLOBAL: LazyLock<RwLock<SecretRegistry>> =
    LazyLock::new(|| RwLock::new(SecretRegistry::new()));

/// Records a value resolved from the environment, process-wide.
///
/// The registry is global because redaction has to hold at every output path —
/// the summary, each Allure attachment, the console, and an error on its way
/// out of `main` — and those are reached from places that never see the
/// environment file the value came from.
pub fn record_secret(value: &str) {
    if let Ok(mut reg) = GLOBAL.write() {
        reg.record(value);
    }
}

/// Replaces every recorded secret in `text`. Cheap when nothing was recorded.
pub fn redact(text: &str) -> String {
    match GLOBAL.read() {
        Ok(reg) if !reg.is_empty() => reg.redact(text),
        _ => text.to_string(),
    }
}

pub fn has_secrets() -> bool {
    GLOBAL.read().map(|r| !r.is_empty()).unwrap_or(false)
}

#[cfg(test)]
pub fn reset_secrets_for_test() {
    if let Ok(mut reg) = GLOBAL.write() {
        *reg = SecretRegistry::new();
    }
}

/// Resolves `${VAR}` and `${VAR:-default}` in every string value of a parsed
/// YAML document, recording each value that came from the environment.
///
/// Mapping keys are left alone: the syntax is defined over values, and a
/// substituted key would silently reshape the document.
pub fn resolve_env_placeholders(value: &mut serde_yaml::Value, file_label: &str) -> Result<(), String> {
    let mut registry = SecretRegistry::new();
    resolve_in_value(value, file_label, &mut registry)?;
    for secret in &registry.values {
        record_secret(secret);
    }
    Ok(())
}

fn resolve_in_value(
    value: &mut serde_yaml::Value,
    file_label: &str,
    registry: &mut SecretRegistry,
) -> Result<(), String> {
    match value {
        serde_yaml::Value::String(s) => {
            let resolved = substitute(s, file_label, registry)?;
            *s = resolved;
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                resolve_in_value(item, file_label, registry)?;
            }
        }
        serde_yaml::Value::Mapping(map) => {
            for (_, v) in map.iter_mut() {
                resolve_in_value(v, file_label, registry)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Expands one string.
///
/// `${NAME}` requires the variable to be set; `${NAME:-fallback}` supplies a
/// literal when it is not. `$${NAME}` escapes the syntax and yields the text
/// `${NAME}` — which is why the escape has to be handled before the
/// placeholder, not after.
fn substitute(input: &str, file_label: &str, registry: &mut SecretRegistry) -> Result<String, String> {
    if !input.contains('$') {
        return Ok(input.to_string());
    }

    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'$' {
            let ch_len = utf8_len(bytes[i]);
            out.push_str(&input[i..i + ch_len]);
            i += ch_len;
            continue;
        }

        // `$${...}` is a literal `${...}`.
        if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            if i < bytes.len() && bytes[i] == b'{' {
                let end = find_close(bytes, i).ok_or_else(|| {
                    format!("unterminated_env_placeholder: '$${{' without a closing '}}' in {file_label}")
                })?;
                out.push_str(&input[i..=end]);
                i = end + 1;
            }
            continue;
        }

        if i + 1 >= bytes.len() || bytes[i + 1] != b'{' {
            out.push('$');
            i += 1;
            continue;
        }

        let end = find_close(bytes, i + 1).ok_or_else(|| {
            format!("unterminated_env_placeholder: '${{' without a closing '}}' in {file_label}")
        })?;
        let body = &input[i + 2..end];
        let (name, default) = match body.find(":-") {
            Some(pos) => (&body[..pos], Some(&body[pos + 2..])),
            None => (body, None),
        };

        if !is_valid_name(name) {
            return Err(format!(
                "invalid_env_placeholder: '${{{body}}}' in {file_label} is not a valid variable name"
            ));
        }

        match std::env::var(name) {
            Ok(resolved) => {
                registry.record(&resolved);
                out.push_str(&resolved);
            }
            Err(_) => match default {
                // A default lives in the file already, so it is not a secret.
                Some(fallback) => out.push_str(fallback),
                None => {
                    return Err(format!(
                        "unresolved_env_var: '${{{name}}}' in {file_label} is not set in the environment \
                         (use '${{{name}:-default}}' to supply a fallback)"
                    ))
                }
            },
        }
        i = end + 1;
    }

    Ok(out)
}

fn find_close(bytes: &[u8], open_brace: usize) -> Option<usize> {
    debug_assert_eq!(bytes[open_brace], b'{');
    (open_brace + 1..bytes.len()).find(|&j| bytes[j] == b'}')
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

/// Parses a speq artifact, resolving `${VAR}` before it is deserialised.
///
/// Substitution has to happen on the parsed document rather than on the raw
/// text: a token may contain `:`, `#` or a newline, and splicing one into YAML
/// source would reshape the document instead of filling a value.
///
/// `on_yaml_err` keeps each caller's existing error wording. Deserialising from
/// a `Value` loses the line and column a `&str` carries, so when the typed
/// parse fails the original text is parsed once more: if that fails too its
/// positioned error is the better one to show, and if it succeeds the failure
/// was introduced by substitution and says so.
pub fn parse_and_resolve<T, F>(content: &str, file_label: &str, on_yaml_err: F) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
    F: Fn(serde_yaml::Error) -> String,
{
    let mut doc: serde_yaml::Value = serde_yaml::from_str(content).map_err(&on_yaml_err)?;
    resolve_env_placeholders(&mut doc, file_label)?;
    match serde_yaml::from_value::<T>(doc) {
        Ok(parsed) => Ok(parsed),
        Err(after) => match serde_yaml::from_str::<T>(content) {
            Err(before) => Err(on_yaml_err(before)),
            Ok(_) => Err(format!(
                "env_substitution_error: {} became invalid in {} after '${{...}}' substitution: {}",
                std::any::type_name::<T>().rsplit("::").next().unwrap_or("document"),
                file_label,
                after
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(input: &str) -> Result<(String, SecretRegistry), String> {
        let mut reg = SecretRegistry::new();
        let out = substitute(input, "demo.yaml", &mut reg)?;
        Ok((out, reg))
    }

    #[test]
    fn a_set_variable_is_substituted_and_recorded_as_secret() {
        std::env::set_var("SPEQ_TEST_TOKEN_A", "s3cr3t-value-aaa");
        let (out, reg) = expand("Bearer ${SPEQ_TEST_TOKEN_A}").expect("resolves");
        assert_eq!(out, "Bearer s3cr3t-value-aaa");
        assert_eq!(reg.values, vec!["s3cr3t-value-aaa".to_string()]);
    }

    #[test]
    fn a_default_is_used_when_unset_and_is_not_a_secret() {
        std::env::remove_var("SPEQ_TEST_ABSENT_B");
        let (out, reg) = expand("${SPEQ_TEST_ABSENT_B:-https://staging.example.com}").expect("resolves");
        assert_eq!(out, "https://staging.example.com");
        assert!(
            reg.is_empty(),
            "a default is written in the file, so it is not a secret to redact"
        );
    }

    #[test]
    fn a_set_variable_wins_over_its_default() {
        std::env::set_var("SPEQ_TEST_PRESENT_C", "from-env-ccc");
        let (out, _) = expand("${SPEQ_TEST_PRESENT_C:-fallback}").expect("resolves");
        assert_eq!(out, "from-env-ccc");
    }

    #[test]
    fn an_empty_default_is_allowed() {
        std::env::remove_var("SPEQ_TEST_ABSENT_D");
        let (out, _) = expand("prefix-${SPEQ_TEST_ABSENT_D:-}").expect("resolves");
        assert_eq!(out, "prefix-");
    }

    #[test]
    fn a_missing_variable_without_a_default_names_itself_and_its_file() {
        std::env::remove_var("SPEQ_TEST_ABSENT_E");
        let err = expand("${SPEQ_TEST_ABSENT_E}").expect_err("must fail loudly");
        assert!(err.starts_with("unresolved_env_var:"), "unexpected: {err}");
        assert!(err.contains("SPEQ_TEST_ABSENT_E"), "must name the variable: {err}");
        assert!(err.contains("demo.yaml"), "must name the file: {err}");
        assert!(err.contains(":-default"), "must point at the fallback form: {err}");
    }

    #[test]
    fn the_double_dollar_escape_yields_a_literal_placeholder() {
        std::env::set_var("SPEQ_TEST_TOKEN_F", "never-used");
        let (out, reg) = expand("$${SPEQ_TEST_TOKEN_F}").expect("resolves");
        assert_eq!(out, "${SPEQ_TEST_TOKEN_F}");
        assert!(reg.is_empty(), "an escaped placeholder reads nothing from the environment");
    }

    #[test]
    fn a_dollar_that_starts_no_placeholder_is_literal() {
        let (out, _) = expand("costs $5 and $ alone").expect("resolves");
        assert_eq!(out, "costs $5 and $ alone");
    }

    #[test]
    fn several_placeholders_in_one_value_all_resolve() {
        std::env::set_var("SPEQ_TEST_USER_G", "alice");
        std::env::set_var("SPEQ_TEST_HOST_G", "api.example.com");
        let (out, reg) = expand("https://${SPEQ_TEST_USER_G}@${SPEQ_TEST_HOST_G}/v1").expect("resolves");
        assert_eq!(out, "https://alice@api.example.com/v1");
        assert_eq!(reg.values.len(), 2);
    }

    #[test]
    fn a_malformed_placeholder_is_rejected() {
        assert!(expand("${}").expect_err("empty name").contains("invalid_env_placeholder"));
        assert!(expand("${9LIVES}").expect_err("leading digit").contains("invalid_env_placeholder"));
        assert!(expand("${has space}").expect_err("space").contains("invalid_env_placeholder"));
        assert!(
            expand("${UNTERMINATED").expect_err("no brace").contains("unterminated_env_placeholder")
        );
    }

    #[test]
    fn redaction_replaces_every_occurrence_longest_first() {
        let mut reg = SecretRegistry::new();
        reg.record("tok");
        reg.record("tok-en-full");
        let out = reg.redact("header=tok-en-full body=tok");
        assert_eq!(
            out,
            format!("header={REDACTED} body={REDACTED}"),
            "the longer secret must not be left as a fragment of the shorter one"
        );
    }

    #[test]
    fn an_empty_value_is_never_recorded() {
        let mut reg = SecretRegistry::new();
        reg.record("");
        assert!(reg.is_empty(), "redacting the empty string would match everywhere and mean nothing");
    }

    #[test]
    fn placeholders_resolve_through_a_whole_document() {
        std::env::set_var("SPEQ_TEST_KEY_H", "hunter2-hhh");
        std::env::remove_var("SPEQ_TEST_ABSENT_H");
        let mut doc: serde_yaml::Value = serde_yaml::from_str(
            "baseUrl: https://api.example.com\n\
             headers:\n  authorization: 'Bearer ${SPEQ_TEST_KEY_H}'\n\
             retries: 3\n\
             hosts:\n  - '${SPEQ_TEST_ABSENT_H:-fallback.example.com}'\n",
        )
        .expect("parses");
        resolve_env_placeholders(&mut doc, "ci.yaml").expect("resolves");

        assert_eq!(
            doc["headers"]["authorization"].as_str(),
            Some("Bearer hunter2-hhh"),
            "a nested mapping value resolves"
        );
        assert_eq!(
            doc["hosts"][0].as_str(),
            Some("fallback.example.com"),
            "a sequence item resolves"
        );
        assert_eq!(doc["retries"].as_u64(), Some(3), "non-string scalars are untouched");
        assert!(redact("token=hunter2-hhh").contains(REDACTED), "the value is now redactable");
    }

    #[test]
    fn a_secret_containing_yaml_syntax_survives_substitution() {
        // The reason substitution runs on the parsed document rather than the
        // raw text: this value would reshape the file if spliced into source.
        std::env::set_var("SPEQ_TEST_GNARLY_I", "a: b #c\nd");
        let mut doc: serde_yaml::Value =
            serde_yaml::from_str("token: '${SPEQ_TEST_GNARLY_I}'\n").expect("parses");
        resolve_env_placeholders(&mut doc, "ci.yaml").expect("resolves");
        assert_eq!(doc["token"].as_str(), Some("a: b #c\nd"));
    }
}
