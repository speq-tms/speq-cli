use jsonschema::JSONSchema;
use serde_json::Value;
use speq_cli::cli::run::{SummaryReport, SummaryTestRecord, SummaryTotals};
use speq_cli::coverage::{CoverageReport, Endpoint};
use std::fs;
use std::path::PathBuf;

/// The vendored copy of the published `speq-contracts` results schema.
///
/// It is a mirror, not an opinion of our own: `scripts/sync-contracts.sh --check`
/// runs in CI and fails if it has drifted from the pinned revision recorded in
/// `tests/fixtures/contracts/CONTRACTS_PIN`. Validating against it here is
/// therefore equivalent to validating against the published contract.
fn results_schema() -> JSONSchema {
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/contracts/results/v1.json");
    let schema_raw = fs::read_to_string(&schema_path).expect("read results schema");
    let schema_json: Value = serde_json::from_str(&schema_raw).expect("parse results schema");
    JSONSchema::compile(&schema_json).expect("compile schema")
}

fn assert_valid(schema: &JSONSchema, case: &str, summary: SummaryReport) {
    let instance = serde_json::to_value(summary).expect("summary to json");
    let details: Vec<String> = match schema.validate(&instance) {
        Ok(()) => return,
        Err(errors) => errors.map(|e| e.to_string()).collect(),
    };
    panic!("summary contract validation failed for {case}: {}", details.join("; "));
}

fn minimal_summary() -> SummaryReport {
    SummaryReport {
        status: "passed".to_string(),
        started_at_ms: 1,
        duration_ms: 5,
        totals: SummaryTotals {
            passed: 1,
            failed: 0,
            total: 1,
            pending: None,
            error: None,
        },
        tests: vec![SummaryTestRecord {
            id: "smoke.health".to_string(),
            status: "passed".to_string(),
            duration_ms: 5,
            message: None,
        }],
        coverage: None,
    }
}

#[test]
fn minimal_summary_matches_results_v1_schema() {
    assert_valid(&results_schema(), "a passing run", minimal_summary());
}

/// Every optional field the runtime can emit must be describable by the
/// contract. The old version of this test exercised only the minimal shape,
/// which is how `totals.pending`, `totals.error` and `coverage` shipped in
/// v1.0.0 without the published schema ever mentioning them.
#[test]
fn fully_populated_summary_matches_results_v1_schema() {
    let summary = SummaryReport {
        status: "failed".to_string(),
        started_at_ms: 1_760_000_000_000,
        duration_ms: 4_210,
        totals: SummaryTotals {
            passed: 12,
            failed: 1,
            total: 17,
            pending: Some(3),
            error: Some(1),
        },
        tests: vec![
            SummaryTestRecord {
                id: "posts.create".to_string(),
                status: "passed".to_string(),
                duration_ms: 120,
                message: None,
            },
            SummaryTestRecord {
                id: "posts.update".to_string(),
                status: "failed".to_string(),
                duration_ms: 98,
                message: Some("expected status 200, got 500".to_string()),
            },
            SummaryTestRecord {
                id: "posts.delete".to_string(),
                status: "pending".to_string(),
                duration_ms: 0,
                message: Some("pending: not yet implemented".to_string()),
            },
            SummaryTestRecord {
                id: "users.list".to_string(),
                status: "error".to_string(),
                duration_ms: 31,
                message: Some("connection refused".to_string()),
            },
        ],
        coverage: Some(CoverageReport {
            enabled: true,
            total_endpoints: 9,
            covered_endpoints: 3,
            percentage: 33.333_333_333_333_33,
            uncovered: vec![
                Endpoint { method: "GET".to_string(), path: "/posts".to_string() },
                Endpoint { method: "DELETE".to_string(), path: "/posts/{id}".to_string() },
            ],
        }),
    };

    assert_valid(&results_schema(), "a run with pending, error and coverage", summary);
}

/// A run with no OpenAPI endpoints reports 100% rather than NaN, and an empty
/// `uncovered` list. NaN would serialise as `null` and fail the contract.
#[test]
fn empty_coverage_matches_results_v1_schema() {
    let mut summary = minimal_summary();
    summary.coverage = Some(CoverageReport {
        enabled: true,
        total_endpoints: 0,
        covered_endpoints: 0,
        percentage: 100.0,
        uncovered: vec![],
    });

    assert_valid(&results_schema(), "a run with an empty OpenAPI spec", summary);
}

/// The contract is closed: an unexpected field must be rejected. Without this,
/// the schema could silently widen to `additionalProperties: true` and the
/// tests above would keep passing while the contract stopped constraining
/// anything.
#[test]
fn results_v1_schema_rejects_unknown_fields() {
    let mut instance = serde_json::to_value(minimal_summary()).expect("summary to json");
    instance
        .as_object_mut()
        .expect("summary is an object")
        .insert("unexpectedField".to_string(), Value::Bool(true));

    assert!(
        results_schema().validate(&instance).is_err(),
        "results/v1.json accepted an unknown top-level field; the contract is no longer closed"
    );
}
