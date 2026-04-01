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
