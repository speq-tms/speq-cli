# Contributing to speq-cli

## Workflow

- Create a branch from `main`:
  - `feat/<scope>-<short-name>` for features
  - `fix/<scope>-<short-name>` for fixes
- Keep pull requests focused and small.
- Add or update tests for behavior changes.

## Commit style

Use Conventional Commit prefixes:

- `feat:`
- `fix:`
- `docs:`
- `refactor:`
- `test:`
- `chore:`

## Pull request checklist

- [ ] Code builds successfully.
- [ ] Tests are added/updated and passing.
- [ ] Docs are updated when behavior changes.
- [ ] No breaking change is introduced without explicit note.

## Runtime rule

Execution logic belongs only to `speq-cli`. Other repositories must call CLI instead of reimplementing runtime.

## Contract rule

`speq-contracts` owns the schemas. `speq-cli` never keeps an opinion of its own about them.

The files under `tests/fixtures/contracts/` are a mirror, vendored only so `cargo test` runs offline.
Never hand-edit them: `scripts/sync-contracts.sh --check` runs in CI and fails when they drift from the
revision pinned in `tests/fixtures/contracts/CONTRACTS_PIN`.

When the runtime starts emitting something the contract does not describe:

1. change the schema in `speq-contracts` and merge it there;
2. move `ref=` in `CONTRACTS_PIN` to the revision that contains the change;
3. run `scripts/sync-contracts.sh` and commit the refreshed mirror alongside the runtime change.
