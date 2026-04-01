# speq-cli Phase 1 backlog

## Goal

Deliver `speq-cli` alpha with stable execution contracts and basic cross-platform distribution.

## P0: Contracts and foundations

1. Speq root discovery and mode detection
   - `in-repo` mode: `.speq/manifest.yaml + suites/`
   - `test-repo` mode: `manifest.yaml + suites/` in repo root
   - explicit override via `--speq-root`
   - hard error for ambiguous structure
2. Freeze v1 output contracts
   - JSON result model
   - exit codes (`0`, `1`, `2`, `3`)
3. CLI command surface baseline
   - `init`, `validate`, `list`, `run`, `report`, `doctor`

## P1: Runtime extraction

4. Parser migration from prototype
   - test YAML parser
   - manifest and environment loading
5. Runner migration from prototype
   - variables and env substitution
   - reusable steps (`type: use`)
   - supported assertions:
     - `status`, `json`, `contains`, `notcontains`, `exists`, `regex`
6. Reporting migration
   - JSON summary output
   - Allure-compatible result artifacts

## P1.5: Migration and compatibility

7. Implement `speq migrate-layout`
   - `.tms_test` -> `.speq`
   - migration hints for dedicated test-repo mode
8. Backward compatibility notes
   - clear user-facing warnings and actionable migration messages

## P2: CI and release readiness

9. Add e2e fixtures
   - happy path and failing assertions
   - config/validation failures
10. CI matrix
    - Linux, macOS, Windows
11. Alpha packaging
    - publish `v0.1.0-alpha` artifacts

## Definition of done (Phase 1)

- Deterministic local vs CI behavior on reference fixtures.
- Exit codes and JSON output covered by tests.
- Allure output generated for successful and failed runs.
- Migration path from legacy `.tms_test` documented and tested.
