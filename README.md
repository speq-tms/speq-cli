# speq-cli

Open-source CLI runtime for `speq`.

## Scope

`speq-cli` is the single execution engine for the platform:

- parse and validate YAML test specs;
- execute test plans in a deterministic way;
- produce machine-readable JSON output and stable exit codes;
- generate Allure-compatible artifacts.

Core runtime is implemented in Rust.

## Current commands

- `speq init --mode in-repo|test-repo`
- `speq list [--speq-root <path>] [--format json]`
- `speq validate [--speq-root <path>] [--format json]`
- `speq run [--speq-root <path>] [--env <name>] [--test <file>|--suite <dir>] [--tags smoke,api] [--report all|summary|allure] [--output <summary.json>]`
- `speq report [--speq-root <path>] [--format all|summary|allure] [--summary <summary.json>]`
- `speq doctor [--speq-root <path>] [--format json]`

`run` report mode defaults to `allure` when `--report` is not set.

## Repository layout

```text
src/
  cli/
  parser/
  manifest/
docs/
```

## Local development

- Build: `cargo build`
- Test: `cargo test`
- Validate canonical fixtures:
  - `cargo run -- validate --speq-root ../speq-examples/in-repo-mode/.speq --format json`
  - `cargo run -- validate --speq-root ../speq-examples/test-repo-mode --format json`

## Status

MVP CLI is functionally complete for OSS alpha:

- init/list/validate/run/report/doctor;
- dual layout support (in-repo/test-repo);
- report outputs (summary/allure/all) with contract regression tests.
