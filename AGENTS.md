# AGENTS.md

> **Documentation rule.** Every change that affects behaviour, configuration, API, build or deployment MUST
> update `README.md` and the relevant pages in `docs/wiki/` in the same commit / pull request. A change is not
> done until the docs match the code.

Instructions for AI coding agents working in this repository.

## Project overview

Currency Exchange Service is a gRPC currency conversion and exchange-rate service in Rust (a Cargo workspace,
tonic + sqlx + PostgreSQL). It converts human-readable amounts to and from integer storage
units (scale `1e9`) and exchanges between currencies at rates that a scheduled `sync` job
loads from OpenExchangeRates, Coinbase and CoinGecko. Conversions are served from an
in-memory snapshot and never touch the database.

- Contract: `proto/currency/v1/currency.proto` (`currency.v1.CurrencyService`).
- Binaries (crate `crates/app`): `currency-manager` (server), `migrate`, `seed`, `sync`.
- Docs: `README.md` and `docs/wiki/*.md` (the GitHub Wiki source).

## Build and test

Requirements: Rust 1.88 (`rust-toolchain.toml`) and `protoc`.

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
docker compose up --build        # full local stack
```

All four cargo commands must pass before you consider a change done.

## Crate boundaries (do not break)

```
app ──► api ──► store ──► core
 │                          ▲
 └──► providers ──► provider-sdk ──┘
```

- `core`: pure arithmetic, error codes and the log format. No I/O.
- `provider-sdk`: the `RateProvider` trait, config and HTTP kit. It must NOT depend on `store`.
- `providers/*`: one crate per vendor, depending only on `provider-sdk`.
  `providers/registry` is the only place adapters are named, each behind a cargo feature.
- `store`: Postgres access and embedded migrations (`sqlx::migrate!`).
- `api`: the gRPC service and the in-memory registry. Handlers never await the database.
- `app`: binaries and Prometheus metrics.

## Conventions observed in the code

- Money is `rust_decimal::Decimal` internally, never `f64`. Rounding is
  `MidpointAwayFromZero`.
- Refusals carry an `ErrorCode` in the `x-error-code` metadata. Do not change the codes,
  metadata keys, metric names, log field names, env var names or their defaults: deployments
  depend on them.
- Adapters omit unknown codes, never return the base currency and propagate errors.
- Every log call has a stable `event = "..."` field. Use `kit::redact` for anything that may
  contain a URL with a key.
- Constants are named, with no magic numbers. Tests are named as sentences and live next to
  the code in `#[cfg(test)] mod tests`.
- Comments explain why, not what. Keep them in English.
- Formatting is plain `rustfmt` (no config file). Clippy runs with `-D warnings`.

## Hard rules

- Never edit an existing file in `migrations/`. Add a new numbered migration.
- Never rename or renumber anything in `proto/`. Add new fields with new numbers.
- Do not add a database call on the conversion path.
- Keep `README.md` and `docs/wiki/` in sync with the code (the documentation rule above).
