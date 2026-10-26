# Contributing

Bug reports, ideas and pull requests are welcome. This page explains how the project is developed and what a change needs before it can be merged.

## Reporting bugs and requesting features

- Open an issue with the **Bug report** form, or a blank issue for anything else.
- Report security problems privately as described in [SECURITY.md](SECURITY.md), not in an issue.

## Development setup

Building requires Rust 1.85 or newer (edition 2024). After cloning, enable the repository's Git hooks once:

```sh
git config core.hooksPath .githooks
```

The `pre-push` hook runs `cargo fmt --check` and rejects the push if formatting differs.

## Checks

minidis has two builds, full (default) and lite (`--no-default-features`). Every change must pass the checks for both, as the CI workflow does:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --no-default-features --all-targets -- -D warnings
cargo test
cargo test --no-default-features
```

Code that only exists in the full build goes behind `#[cfg(feature = "full")]`, and so do tests that use lists, hashes or expiry.

## Branches and commits

- Work on a short branch named `type/subject`, for example `feat/graceful-shutdown` or `fix/flaky-test-port`.
- Commit messages are one short line starting with a type: `feat:`, `fix:`, `test:`, `docs:`, `ci:`, `chore:` or `refactor:`. A scope in parentheses is not used.
- Keep one topic per branch.

## Pull requests

A pull request is ready for review when:

- The checks above pass for both builds
- New behavior is covered by tests
- The README, the Helm values and the configuration table are updated if behavior or settings changed

## License

Unless explicitly stated otherwise, any contribution intentionally submitted for inclusion in this project, as defined in the Apache-2.0 license, is dual licensed under the MIT and Apache-2.0 licenses, without any additional terms or conditions.
