# speq-cli

Core runtime for SPEQ written in Rust. The single execution engine of the ecosystem.

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
- Runtime logic must stay in this repository, never in the extension or the runner.
- Validation behavior must remain aligned with `speq-contracts`. When the DSL changes, `speq-cli`,
  `speq-contracts`, and `speq-vscode-extension` are updated in the same release candidate.
- This repository is the source of truth for CLI behavior. Documentation follows the code; when
  `speq-docs` and this code disagree, the code is right and the document is a bug.

## Local debugging and AT guidelines

### Local CLI debugging
Build the release binary with `cargo build --release`, then run the e2e suite in
`speq-examples/test-repo-mode-jsonplaceholder` using `speq run --env ci`. This is the primary way to verify
runtime correctness end-to-end against a real API (JSONPlaceholder). Always clean `reports/allure/` and
`reports/results/` before each run to avoid stale output.

### Backward compatibility check
After any change here, all existing tests in `speq-examples/test-repo-mode-jsonplaceholder` must remain green.
No regressions are allowed. A passing run shows `"failed": 0` in the JSON summary output.

### New feature AT coverage
Every new feature must be covered by at least one acceptance test (example) in
`speq-examples/test-repo-mode-jsonplaceholder`, demonstrating the feature in a realistic scenario against the
JSONPlaceholder API. This applies to new features only — bug fixes and improvements to existing features do
not require new AT examples.

## How we work

Full process: `speq-docs/docs/delivery/release-flow.md`. Read it before starting delivery work. Summary:

- **Issues live in [`speq-tms/speq-docs`](https://github.com/speq-tms/speq-docs/issues)**, not here. Work for
  this repository carries the `area/cli` label.
- **Milestone title == RC branch name.** Milestone `v1.1.0` means branch `v1.1.0`. `backlog` is not a release
  and has no branch.
- **Find the current RC** — GitHub state is authoritative, not any version written in a file:

  ```bash
  gh api repos/speq-tms/speq-docs/milestones \
    --jq '.[] | select(.state=="open" and .title != "backlog") | .title'
  git ls-remote --heads origin 'v*'
  ```

- **Branch from the RC, never from `main`:** `git switch -c feat/cli-<name> origin/<RC>`.
- **PR base is the RC**, never `main`. One final PR takes the RC into `main`.
- `Closes #N` does **not** work across repositories. Write `Part of speq-tms/speq-docs#N` in the PR, then close
  the issue manually after merge:
  `gh issue close N --repo speq-tms/speq-docs --comment "Landed in <PR url>."`

> **Release trap.** `.github/workflows/release.yml` triggers a release build when a PR whose head branch name
> **starts with `v`** is merged into `main`. Never name a branch that way unless a release is intended. Small
> non-release changes — agent instructions, editor config — go through a `chore/*` branch straight into `main`;
> that path does not trigger the workflow.

### Current state (verify with the commands above)
Released: `v1.0.0`. There is **no** RC branch here — the `v1.1.0` milestone deliberately contains no runtime
work; it covers documentation, contracts, and examples. Known runtime issues sit in the `backlog` milestone:
`speq-tms/speq-docs#1` (`coverage.fail_below` naming) and `#2` (parallel execution specified but not built).
Before starting runtime work, open an RC branch here and move the issues into that milestone.
