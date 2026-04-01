# speq-cli

Open-source CLI runtime for `speq`.

## Scope

`speq-cli` is the single execution engine for the platform:

- parse and validate YAML test specs;
- execute test plans in a deterministic way;
- produce machine-readable JSON output and stable exit codes;
- generate Allure-compatible artifacts.

## Planned commands (v1)

- `speq init`
- `speq validate`
- `speq list`
- `speq run`
- `speq report`
- `speq doctor`
- `speq migrate-layout`

## Repository layout

```text
src/
  cli/
  parser/
  runner/
  assertions/
  manifest/
  env/
  reporting/
schemas/
examples/
docs/
tests/
```

## Status

Bootstrap complete. Phase 1 implementation is tracked in `docs/PHASE1_BACKLOG.md`.
