# Configuration

All configuration comes from environment variables. Nothing is read from files.

## Core

| Variable        | Default              | Used by            | Description |
|-----------------|----------------------|--------------------|-------------|
| `DATABASE_URL`  | required             | all four binaries  | PostgreSQL connection string, e.g. `postgres://user:pass@host:5432/db`. |
| `GRPC_PORT`     | `9090`               | `currency-manager` | gRPC port. A bare number, not an address; the server binds `0.0.0.0`. An unparsable value falls back to the default. |
| `HTTP_PORT`     | `8080`               | `currency-manager` | Port of the HTTP liveness probe `GET /__health`. |
| `METRICS_PORT`  | `9464`               | `currency-manager` | Port of Prometheus `GET /metrics`. If it cannot bind, the server logs `metrics.failed` and keeps serving conversions. |
| `BASE_CURRENCY` | `EUR`                | `sync`             | The currency every rate is quoted against. It is upper-cased, and an empty value means the default. At the end of every run it is pinned to `1`. The seed registers `EUR`, so change this only together with your registry. |
| `RUST_LOG`      | `info,currency=info` | all four binaries  | [`tracing-subscriber` env-filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) directives. |
| `OPERATOR`      | `aggregator`         | all four binaries  | Written as the `env` field of every log line, so you can tell deployments apart. |

## Rate providers

The `sync` job reads provider settings by prefix. The prefix is the provider key in upper
case (`OPENEXCHANGE_`, `COINBASE_`, `COINGECKO_`). The rest of the variable name, lower-cased,
becomes the setting name. **An empty or whitespace-only value counts as unset.** This matters
in Kubernetes, where an unfilled secret key arrives as an empty string.

| Variable                | Default                             | Description |
|-------------------------|-------------------------------------|-------------|
| `OPENEXCHANGE_APP_ID`   | none (required for fiat)            | OpenExchangeRates app id. Without it the adapter refuses with a "config is missing app_id" error and `sync` exits non-zero. |
| `OPENEXCHANGE_BASE_URL` | `https://openexchangerates.org/api` | API base URL. Useful for tests and proxies. |
| `COINBASE_BASE_URL`     | `https://api.coinbase.com/v2`       | API base URL. Coinbase needs no key. |
| `COINGECKO_API_KEY`     | none                                | Demo API key, sent as the `x-cg-demo-api-key` header. Keyless calls from many cloud/datacenter IP ranges get HTTP 403. |
| `COINGECKO_BASE_URL`    | `https://api.coingecko.com/api/v3`  | API base URL. |
| `COINGECKO_MAPPING`     | none                                | Extra `CODE:coin-id` pairs, comma-separated, e.g. `TON:the-open-network,XYZ:xyz-coin`. Codes are upper-cased, and malformed entries are dropped. Entries override the built-in table. |

The adapter's secret never reaches the logs. Query strings are stripped from every URL in
error messages and vendor call logs, because OpenExchangeRates takes its key as a query
parameter.

## Fixed values

These are compile-time constants. Changing one is a code change and must follow the
documentation rule.

| What | Value | Where |
|------|-------|-------|
| Registry refresh interval | 60 s | `crates/api/src/rates.rs` (`REFRESH_INTERVAL`) |
| Stale-rate threshold (logs `WARN`, still served) | 24 h | `crates/api/src/rates.rs` (`STALE_AFTER`) |
| Server DB pool size | 2 | `crates/app/src/main.rs` (`DB_CONNECTIONS`) |
| gRPC call timeout | 5 s | `crates/app/src/main.rs` (`GRPC_CALL_TIMEOUT`) |
| gRPC HTTP/2 keepalive interval / timeout | 30 s / 10 s | `crates/app/src/main.rs` |
| Vendor request / connect timeout | 15 s / 5 s | `crates/provider-sdk/src/kit.rs` |
| Units scale used by the seed | `1e9` | `crates/app/src/bin/seed.rs` (`NANO`) |
