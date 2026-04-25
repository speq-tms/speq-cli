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

## DSL highlights (alpha.2)

New runtime DSL capabilities implemented in CLI:

- `assert` now supports `type: schema` with `ref` (from `schemasDir`) or `inline`.
- Modules support native `use action` with `imports`.
- `use` steps support `properties` payload to pass per-call parameters into module actions.
- `init.yaml` supports suite hooks and imports:
  - `suite.beforeAll`, `suite.beforeEach`, `suite.afterEach`, `suite.afterAll`
  - `suite.variables`
  - `suite.imports` (available in hooks and inherited by suite tests)

Action contract format in module files:

```yaml
actions:
  getPostById:
    properties: [postId]
    steps:
      - type: api
        name: "GET /posts/{{postId}}"
        method: GET
        url: "/posts/{{postId}}"
```

Use from test or hook:

```yaml
- type: use
  name: "Get post by id"
  action: "jp.getPostById"
  properties:
    postId: "{{targetPostId}}"
```

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

## Installation (Homebrew)

```bash
brew tap speq-tms/tap https://github.com/speq-tms/homebrew-tap
brew install speq
```

## Release packaging

Tag-driven release assets are built by `.github/workflows/release.yml`:

- `speq-linux-x86_64.tar.gz`
- `speq-darwin-x86_64.tar.gz`
- `speq-darwin-aarch64.tar.gz`
- matching `.sha256` files for each archive

Homebrew formula updates are documented in `docs/HOMEBREW_RELEASE.md`.

## Status

MVP CLI is functionally complete for OSS alpha:

- init/list/validate/run/report/doctor;
- dual layout support (in-repo/test-repo);
- report outputs (summary/allure/all) with contract regression tests.
