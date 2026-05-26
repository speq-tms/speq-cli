# speq-cli

Core runtime for SPEQ written in Rust.

## Responsibilities
- Parse and validate SPEQ DSL.
- Execute scenarios and modules.
- Produce reports and machine-readable outputs.

## Commands
- `cargo build`
- `cargo test`
- `cargo run -- validate --speq-root ../speq-examples/in-repo-mode/.speq --format json`

## Invariants
- Keep backward compatibility for existing DSL unless explicitly planned.
- Runtime logic must stay in this repository (not in extension/runner).
- Validation behavior must remain aligned with `speq-contracts`.

## Local debugging and AT guidelines

### Local CLI debugging
Build the release binary with `cargo build --release`, then run the e2e suite in `speq-examples/test-repo-mode-jsonplaceholder` using `speq run --environment ci`. This is the primary way to verify runtime correctness end-to-end against a real API (JSONPlaceholder). Always clean `reports/allure/` and `reports/results/` before each run to avoid stale output.

### Backward compatibility check
After any change to `speq-cli`, all existing tests in `speq-examples/test-repo-mode-jsonplaceholder` must remain green. No regressions are allowed. A passing run shows `"failed": 0` in the JSON summary output.

### New feature AT coverage
Every new feature delivered as part of a Phase must be covered by at least one acceptance test (example) in `speq-examples/test-repo-mode-jsonplaceholder`. The example should demonstrate the feature using a realistic scenario against the JSONPlaceholder API. This requirement applies only to new features — bug fixes and improvements to existing features do not require new AT examples.
