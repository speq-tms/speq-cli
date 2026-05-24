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
