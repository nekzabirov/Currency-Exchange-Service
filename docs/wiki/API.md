# API

## gRPC

Package `currency.v1`, service `CurrencyService`. The contract lives in
[`proto/currency/v1/currency.proto`](https://github.com/nekzabirov/ratekeeper/blob/main/proto/currency/v1/currency.proto).
The server listens on `GRPC_PORT` (default `9090`) in plaintext and does not enable reflection.

```proto
service CurrencyService {
  rpc ConvertToUnits(ConvertToUnitsRequest) returns (UnitsResponse);
  rpc ConvertFromUnits(ConvertFromUnitsRequest) returns (AmountResponse);
  rpc ConvertUnits(ConvertUnitsRequest) returns (UnitsResponse);
}
```

### ConvertToUnits

A human-readable amount becomes storage units. Only the currency's scale is used. The rate
plays no part.

| Field      | Type     | Notes |
|------------|----------|-------|
| `amount`   | `double` | May be negative. NaN and infinity are refused. |
| `currency` | `string` | Matched case-insensitively, with surrounding whitespace trimmed. |
| → `units`  | `int64`  | `round_half_up(amount × units_scale)`. |

### ConvertFromUnits

Storage units become a human-readable amount.

| Field       | Type     |
|-------------|----------|
| `units`     | `int64`  |
| `currency`  | `string` |
| → `amount`  | `double` |

### ConvertUnits

An exchange: units of `currency` become units of `to_currency` at the current rates.

| Field         | Type     | Notes |
|---------------|----------|-------|
| `units`       | `int64`  | |
| `currency`    | `string` | Source currency. |
| `to_currency` | `string` | Target currency. |
| → `units`     | `int64`  | `round_half_up(units × (rate_to × scale_to) / (rate_from × scale_from))` |

Both currencies must exist in the registry. If they are the same (case-insensitive), `units`
is returned unchanged, even when no rate has ever been loaded. Otherwise both currencies must
have received a rate at least once, or the call fails with `RATE_MISSING`. See
[Money Model](Money-Model) for the arithmetic.

### Request metadata

| Key            | Purpose |
|----------------|---------|
| `x-request-id` | Optional trace id. It is logged as `traceId` on every line the call produces. |

### Errors

| Situation                                       | gRPC status           | `x-error-code` |
|-------------------------------------------------|-----------------------|----------------|
| Currency not in the registry                    | `FAILED_PRECONDITION` | `UNKNOWN_CURRENCY` |
| Rate never loaded (`ConvertUnits`)              | `FAILED_PRECONDITION` | `RATE_MISSING` |
| Non-finite amount or a result outside `int64`   | `INVALID_ARGUMENT`    | `INVALID_AMOUNT` |
| Call exceeded the 5 s server timeout            | `CANCELLED`           | none |

See [Error Codes](Error-Codes) for the full vocabulary and who fixes what.

### Examples

```bash
P="-plaintext -import-path proto -proto currency/v1/currency.proto"

grpcurl $P -d '{"amount":10.50,"currency":"UAH"}' \
  localhost:9090 currency.v1.CurrencyService/ConvertToUnits
# {"units":"10500000000"}

grpcurl $P -d '{"units":"1000000000","currency":"EUR","to_currency":"USDT"}' \
  localhost:9090 currency.v1.CurrencyService/ConvertUnits

grpcurl $P -H 'x-request-id: demo-1' -d '{"amount":1,"currency":"XXX"}' \
  localhost:9090 currency.v1.CurrencyService/ConvertToUnits
# ERROR: Code: FailedPrecondition, Message: currency XXX is not in the registry
```

Generate clients from the `.proto` file with your usual toolchain (`protoc`, `buf`, `tonic-build`
and so on).

## HTTP

| Port           | Method and path   | Response |
|----------------|-------------------|----------|
| `HTTP_PORT`    | `GET /__health`   | `200 ok`. Liveness only; it does not check the database. |
| `METRICS_PORT` | `GET /metrics`    | Prometheus text format `0.0.4`. |

The service has no other REST or WebSocket endpoints.

The probe is plain HTTP rather than the gRPC health protocol. Kubernetes `grpc` probes treat
`UNIMPLEMENTED` as a failure, and the server does not implement `grpc.health.v1.Health`.

## Logs

Every binary writes one JSON object per line to stdout:

```json
{"code":"UNKNOWN_CURRENCY","env":"aggregator","event":"convert.refused","level":"WARN",
 "logger":"currency_api::grpc","msg":"currency XXX is not in the registry","rpc":"ConvertToUnits",
 "service":"currency-manager","traceId":"demo-1","ts":"2026-01-01T12:00:00.000Z"}
```

The example is wrapped here; real output is one line per event, with keys in alphabetical order.

| Field     | Meaning |
|-----------|---------|
| `ts`      | UTC timestamp, ISO-8601, millisecond precision |
| `level`   | `TRACE` to `ERROR` |
| `service` | `currency-manager`, `currency-migrate`, `currency-seed` or `currency-sync` |
| `env`     | value of `OPERATOR` (default `aggregator`) |
| `msg`     | human-readable message |
| `logger`  | Rust module path of the event |
| `rpc`, `traceId` | call span fields (gRPC server only) |
| `event`   | stable machine-readable event name, listed below |

Event names: `startup`, `shutdown.signal`, `shutdown.complete`, `convert.refused`,
`rate.stale`, `registry.empty`, `registry.refreshed`, `registry.refresh_failed`,
`metrics.failed`, `migrate.started`, `migrate.finished`, `seed.started`, `seed.inserted`,
`seed.finished`, `sync.started`, `sync.provider_done`, `sync.provider_failed`,
`sync.provider_unknown`, `sync.finished`, `vendor.call`.

`vendor.call` logs the provider, method, host and path (never the query string), the HTTP
status and the duration in ms.

## Metrics

Served by `currency-manager` on `METRICS_PORT` (default `9464`).

| Metric | Type | Labels | Meaning |
|--------|------|--------|---------|
| `grpc_server_processing_duration_seconds` | histogram | `service`, `method`, `statusCode` | Duration of every gRPC call. `statusCode` uses grpc-java names (`OK`, `FAILED_PRECONDITION` and so on). Paths the server does not serve are labelled `unknown`/`unknown`. Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30. |
| `currency_rates_refresh_total` | counter | `outcome` = `ok` / `failed` | Registry refreshes from the database. Both series exist from start at zero. |
| `db_pool_connections_active` | gauge | | Connections in use. |
| `db_pool_connections_idle` | gauge | | Idle connections. |
| `db_pool_connections_max` | gauge | | Pool ceiling. |
| `process_*` | various | | Standard process metrics: CPU, memory, file descriptors, threads and start time. |
