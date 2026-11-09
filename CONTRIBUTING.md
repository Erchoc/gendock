# Contributing

Small, focused pull requests are welcome. Read AGENTS.md and docs/architecture.md first. Discuss new dependencies, public command changes and architectural changes in an issue before investing in a large implementation.

## Local checks

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

Before 1.0, edit the single `migrations/001_initial.sql` schema snapshot instead of adding upgrade scripts. Existing incompatible development DBs are rejected without resetting user data; versioned migrations start after the released 1.0 baseline.

Use temporary directories for database and CLI checks. Include meaningful tests for persistence, reference ownership, path handling and concurrent claims when changing them. Real model downloads are opt-in integration checks, not unit-test prerequisites.

## Pull requests

Describe the problem, resulting behavior and validation. For capability changes, update [docs/features.md](docs/features.md) in the same PR: status, readiness percentage, verified scope, blockers and next step. These are scoped engineering estimates, not test-coverage percentages; command stubs remain 0%. Mention schema or CLI compatibility changes and update documentation. Keep Cargo.lock committed. Avoid unrelated formatting and generated media. Do not include credentials or user paths in logs/screenshots. Respect third-party model and code licenses.

There is no promise of API stability before 1.0. Even so, document breaking changes. Maintainers should update the repository maintenance skill when a fix establishes a durable lesson.
