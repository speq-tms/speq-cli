use jsonschema::JSONSchema;
use serde_json::Value;
use speq_cli::cli::run::{SummaryReport, SummaryTestRecord, SummaryTotals};
use std::fs;
use std::path::PathBuf;

#[test]
fn summary_report_matches_results_v1_schema() {
    let schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../speq-contracts/schemas/results/v1.json");
    let schema_raw = fs::read_to_string(&schema_path).expect("read results schema");
    let schema_json: Value = serde_json::from_str(&schema_raw).expect("parse results schema");

    let compiled = JSONSchema::compile(&schema_json).expect("compile schema");

    let summary = SummaryReport {
        status: "passed".to_string(),
        started_at_ms: 1,
        duration_ms: 5,
        totals: SummaryTotals {
            passed: 1,
            failed: 0,
            total: 1,
        },
        tests: vec![SummaryTestRecord {
            id: "smoke.health".to_string(),
            status: "passed".to_string(),
            duration_ms: 5,
            message: None,
        }],
    };
    let instance = serde_json::to_value(summary).expect("summary to json");

    let result = compiled.validate(&instance);
    if let Err(errors) = result {
        let details: Vec<String> = errors.map(|e| e.to_string()).collect();
        panic!("summary contract validation failed: {}", details.join("; "));
    }
}
