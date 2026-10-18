# Contributing to ratekeeper

Thanks for your interest in improving ratekeeper. This guide covers how to build, test and
submit changes.

> **Documentation rule.** Every change that affects behaviour, configuration, API, build or deployment MUST
> update `README.md` and the relevant pages in `docs/wiki/` in the same commit / pull request. A change is not
> done until the docs match the code.

## Prerequisites

- Rust 1.88. It is pinned in `rust-toolchain.toml`, and rustup installs it automatically.
- `protoc` (`apt install protobuf-compiler` or `brew install protobuf`).
- Docker with Compose v2 (optional), to run the full stack.

## Build and test

Run all four before opening a pull request. CI runs the same commands.

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The tests need neither a database nor external network access. To try your change end to end:

```bash
docker compose up --build
```

## Pull requests

1. Fork the repository and create a branch from `main`.
2. Keep each pull request focused on one change. Use small, logical commits with clear
   English messages.
3. Add or update tests for any behaviour change.
4. Update `README.md` and the relevant pages in `docs/wiki/` (the documentation rule above).
5. Fill in the pull request checklist.

## Ground rules

- **Never edit an existing migration** in `migrations/`. Add a new numbered file instead,
  because sqlx checksums applied migrations.
- **Keep the gRPC contract backwards compatible.** Do not rename or renumber anything in
  `proto/`.
- **No `f64` money arithmetic.** Use `rust_decimal::Decimal` (see `crates/core/src/money.rs`).
- **Adapters never touch the database.** They depend only on `currency-provider-sdk`.
- **Secrets never reach logs.** Route vendor calls through `kit::send` and errors through
  `kit::redact`.
- Comments explain why, not what.

## Adding a rate provider

See [Rate Providers](https://github.com/nekzabirov/ratekeeper/wiki/Rate-Providers) in the wiki.

## Wiki

The GitHub Wiki is generated from `docs/wiki/`. Edit the Markdown files there. Changes made
directly in the wiki UI are overwritten on the next sync. Wiki links use the page name
without an extension, e.g. `[Configuration](Configuration)`.

## License

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE).
