<div align="center">

# ratekeeper

**Fast gRPC currency conversion and exchange-rate service in Rust.**

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.88-orange.svg?logo=rust)](rust-toolchain.toml)
[![CI](https://github.com/nekzabirov/ratekeeper/actions/workflows/ci.yml/badge.svg)](https://github.com/nekzabirov/ratekeeper/actions/workflows/ci.yml)

</div>

## Why

Money is stored as integers. Systems that move money still have to turn `10.50 UAH` into
storage units and back, and exchange one currency for another at a current rate. They need
to get this right every time, in microseconds, on the hot path of every transaction.

ratekeeper does that one job. It keeps a small currency registry in Postgres, refreshes
exchange rates from public vendors on a schedule, and serves conversions from an in-memory
snapshot. A conversion request never touches the database.

## Features

- **Three gRPC calls.** `ConvertToUnits` and `ConvertFromUnits` change the representation
  of one amount. `ConvertUnits` exchanges between currencies at the current rate.
- **Exact arithmetic.** All math runs in `rust_decimal`, never in binary floating point.
  Rounding is half-up (away from zero), and results that overflow `int64` are refused.
- **No database on the request path.** The registry is held in memory behind an `ArcSwap`
  snapshot and re-read every 60 seconds. Readers never block.
- **Pluggable rate providers.** OpenExchangeRates (fiat), Coinbase (crypto, no key) and
  CoinGecko (crypto). A new provider is one crate plus one line in the registry.
- **Machine-readable failures.** Every refusal carries a stable code in the `x-error-code`
  gRPC metadata key: `UNKNOWN_CURRENCY`, `RATE_MISSING`, `INVALID_AMOUNT` or `INTERNAL`.
- **Stale rates are served, not refused.** A rate older than 24 hours is still served, and
  every such answer logs a `WARN` with the rate's age.
- **Operable.** JSON logs with request tracing through `x-request-id`, Prometheus metrics on
  a dedicated port, a plain HTTP liveness probe and graceful shutdown on SIGTERM.
- **Separate binaries for separate jobs:** server, migrate, seed and sync. A failing sync
  never takes the server down.

## Architecture

```mermaid
flowchart LR
    client([gRPC client]) -->|ConvertToUnits / ConvertFromUnits / ConvertUnits| server
    subgraph server [currency-manager]
        api[gRPC :9090] --> snapshot[(in-memory registry)]
        health[HTTP :8080 /__health]
        metrics[HTTP :9464 /metrics]
    end
    snapshot -. refresh every 60s .-> pg[(PostgreSQL<br/>table: currency)]
    migrate[migrate] -->|schema| pg
    seed[seed] -->|insert-only registry rows| pg
    sync[sync, hourly] -->|rates| pg
    sync --> oxr[OpenExchangeRates]
    sync --> cb[Coinbase]
    sync --> cg[CoinGecko]
```

Workspace crates and their dependencies:

```
app ──► api ──► store ──► core
 │                          ▲
 └──► providers ──► provider-sdk ──┘
```

The rate adapters depend only on `provider-sdk`, so they cannot reach the database.
See [Architecture](https://github.com/nekzabirov/ratekeeper/wiki/Architecture) in the wiki
for the design decisions.

## Quick start

Requirements: Docker with Compose v2.

```bash
git clone https://github.com/nekzabirov/ratekeeper.git
cd ratekeeper
docker compose up --build
```

This starts Postgres, applies the schema (`migrate`), seeds five currencies (`seed`) and
starts the server. Host ports:

| Port    | Purpose                    |
|---------|----------------------------|
| `59090` | gRPC                       |
| `58080` | HTTP liveness `/__health`  |
| `59464` | Prometheus `/metrics`      |
| `55432` | Postgres                   |

Try it:

```bash
curl localhost:58080/__health        # ok
curl localhost:59464/metrics

grpcurl -plaintext -import-path proto -proto currency/v1/currency.proto \
  -d '{"amount":10.50,"currency":"UAH"}' \
  localhost:59090 currency.v1.CurrencyService/ConvertToUnits
# {"units":"10500000000"}
```

`ConvertToUnits` and `ConvertFromUnits` work right after seeding. `ConvertUnits` needs
rates, so run the sync once:

```bash
OPENEXCHANGE_APP_ID=<your-app-id> docker compose run --rm sync
```

Without `OPENEXCHANGE_APP_ID` the fiat adapter refuses and the run exits non-zero. Crypto
rates from Coinbase are still written, because Coinbase needs no key.

## Configuration

All configuration comes from environment variables. An empty provider value
(`OPENEXCHANGE_*`, `COINGECKO_*`, `COINBASE_*`) counts as unset.

| Variable              | Default                              | Used by           | Description |
|-----------------------|--------------------------------------|-------------------|-------------|
| `DATABASE_URL`        | required                             | all binaries      | Postgres connection string. |
| `GRPC_PORT`           | `9090`                               | `currency-manager`| gRPC port. A bare port number, not an address; binds `0.0.0.0`. |
| `HTTP_PORT`           | `8080`                               | `currency-manager`| Port for the `/__health` liveness probe. |
| `METRICS_PORT`        | `9464`                               | `currency-manager`| Port for Prometheus `/metrics`. |
| `BASE_CURRENCY`       | `EUR`                                | `sync`            | Currency all rates are quoted against. It is pinned to `1` at the end of each run. The seed uses `EUR`. |
| `RUST_LOG`            | `info,currency=info`                 | all binaries      | `tracing` env-filter directives. |
| `OPERATOR`            | `aggregator`                         | all binaries      | Value of the `env` field in every log line. |
| `OPENEXCHANGE_APP_ID` | none                                 | `sync`            | OpenExchangeRates app id. Required for fiat rates. |
| `OPENEXCHANGE_BASE_URL` | `https://openexchangerates.org/api` | `sync`           | Override of the OpenExchangeRates API URL. |
| `COINBASE_BASE_URL`   | `https://api.coinbase.com/v2`        | `sync`            | Override of the Coinbase API URL. No key is needed. |
| `COINGECKO_API_KEY`   | none                                 | `sync`            | CoinGecko demo API key, sent as `x-cg-demo-api-key`. Keyless calls from many cloud IP ranges get HTTP 403. |
| `COINGECKO_BASE_URL`  | `https://api.coingecko.com/api/v3`   | `sync`            | Override of the CoinGecko API URL. |
| `COINGECKO_MAPPING`   | none                                 | `sync`            | Extra `CODE:coin-id` pairs, comma-separated (e.g. `TON:the-open-network`). Entries override the built-in table. |

See [Configuration](https://github.com/nekzabirov/ratekeeper/wiki/Configuration) for details.

## API overview

gRPC service `currency.v1.CurrencyService`, defined in
[`proto/currency/v1/currency.proto`](proto/currency/v1/currency.proto):

| RPC                | Request                             | Response          | Needs a synced rate |
|--------------------|-------------------------------------|-------------------|---------------------|
| `ConvertToUnits`   | `amount` (double), `currency`       | `units` (int64)   | no |
| `ConvertFromUnits` | `units` (int64), `currency`         | `amount` (double) | no |
| `ConvertUnits`     | `units`, `currency`, `to_currency`  | `units` (int64)   | yes |

- Currency codes are matched case-insensitively.
- Every currency uses a scale of `1e9`, so `1.0` equals `1000000000` units.
- Send `x-request-id` in the call metadata to have it logged as `traceId`.
- Refusals carry `x-error-code` in the metadata. See
  [Error Codes](https://github.com/nekzabirov/ratekeeper/wiki/Error-Codes).

HTTP: `GET /__health` on `HTTP_PORT` returns `ok`, and `GET /metrics` on `METRICS_PORT`
returns Prometheus text. The service has no other REST or WebSocket endpoints.

## Project structure

```
crates/
  core/                   units/rate arithmetic, error codes, JSON log format
  provider-sdk/           RateProvider trait, ProviderConfig, HTTP kit (timeouts, redaction)
  providers/
    openexchange/         fiat rates, renormalised from USD to the base
    coinbase/             crypto rates, no key required
    coingecko/            crypto rates, inverted vendor quotes
    registry/             the only place adapters are named (cargo features)
  store/                  Postgres pool, `currency` table, embedded migrations
  api/                    gRPC service, in-memory registry, /__health
  app/                    binaries: currency-manager, migrate, seed, sync; Prometheus metrics
migrations/               SQL migrations (sqlx)
proto/                    gRPC contract
docs/wiki/                source of the GitHub Wiki
```

## Development

Requirements: Rust 1.88 (pinned in `rust-toolchain.toml`, so rustup installs it
automatically) and `protoc` (`apt install protobuf-compiler` or `brew install protobuf`).

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The tests need neither a database nor external network access. Running the binaries against a local database:

```bash
docker compose up -d postgres
export DATABASE_URL=postgres://currency:currency@localhost:55432/currency
cargo run --bin migrate
cargo run --bin seed
cargo run --bin currency-manager
```

## Documentation

The full documentation lives in the [Wiki](https://github.com/nekzabirov/ratekeeper/wiki).
Its source is in [`docs/wiki/`](docs/wiki). It covers getting started, configuration,
architecture, the API, deployment, rate providers, the money model and error codes.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md).

> **Documentation rule.** Every change that affects behaviour, configuration, API, build or deployment MUST
> update `README.md` and the relevant pages in `docs/wiki/` in the same commit / pull request. A change is not
> done until the docs match the code.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See also [NOTICE](NOTICE).

---

<sub>Used in production at [1638.cloud](https://1638.cloud).</sub>
