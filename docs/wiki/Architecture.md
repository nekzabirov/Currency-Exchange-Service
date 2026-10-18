# Architecture

## What the service computes

Two DIFFERENT things, and they must not be confused:

| Field         | Meaning | Depends on the rate |
|---------------|---------|---------------------|
| `units_scale` | How many storage units make one human-readable unit | no |
| `rate`        | How many units of the currency one base unit (EUR) buys | yes, this is the exchange rate |

`ConvertToUnits` and `ConvertFromUnits` change the REPRESENTATION of one amount. They work
even before any rate has been loaded. `ConvertUnits` is an EXCHANGE. It refuses if a rate has
never arrived, because converting one-to-one would silently hand out wrong money.

Today the scale is `1e9` ("nano") for every currency, fiat and crypto alike. It is a column
per currency so that it can one day change with a database row rather than a release of
every consumer. See [Money Model](Money-Model).

## Crates

```
app ──► api ──► store ──► core
 │                          ▲
 └──► providers ──► provider-sdk ──┘
```

| Crate                    | Contains | Does NOT know about |
|--------------------------|----------|---------------------|
| `core`                   | unit and rate arithmetic, error codes, the JSON log format | the database, the network, tonic |
| `provider-sdk`           | the `RateProvider` trait, `ProviderConfig`, `kit` (HTTP client, redaction) | **`store`**: an adapter physically cannot touch the table |
| `providers/openexchange` | fiat rates, renormalised from USD to the base | the other adapters |
| `providers/coinbase`     | crypto rates (serves `USDT`, `BTC`, `USDC`), no key | the other adapters |
| `providers/coingecko`    | crypto rates, inverts the vendor's quote | the other adapters |
| `providers/registry`     | the only place adapters are named, each behind a cargo feature | how they work |
| `store`                  | the `currency` table, the pool, embedded migrations | tonic, HTTP |
| `api`                    | the gRPC service, `/__health`, the in-memory registry | the rate adapters |
| `app`                    | the four binaries and the Prometheus exporter | nothing is hidden from it |

## Binaries

| Binary             | Role | Typical deployment |
|--------------------|------|--------------------|
| `currency-manager` | gRPC `:9090`, liveness `:8080`, `/metrics` `:9464` | long-running Deployment |
| `migrate`          | applies the schema (sqlx, advisory-locked, checksummed) | Job, before the server rolls out |
| `seed`             | inserts the registry rows, insert-only | Job, after `migrate` |
| `sync`             | walks the adapters and writes rates | CronJob, hourly |

They are separate because each has its own fate. A failed sync must not take the server down,
and the migration must finish BEFORE the server starts. An application that migrates itself
cannot be rolled back, because the old image would already face the new schema.

## Why a request never touches the database

The service sits on the hot path of money movements. Every transaction may pass through a
scale conversion. A Postgres query on each call would add latency and a new point of failure.

The data is a handful of rows that change once an hour. A background task re-reads the whole
table every 60 seconds and swaps the snapshot with a single pointer (`ArcSwap`), so readers
never block. A failed refresh keeps the previous snapshot: an unreachable database is no
reason to stop converting with numbers already in memory.

The snapshot is populated BEFORE the gRPC port opens. An empty registry would answer
`UNKNOWN_CURRENCY` to every call while still passing the liveness probe.

## The sync run

1. Group the registry's currencies by provider key (`currency.provider`).
2. Ask each provider once for all of its currencies.
3. Store whatever came back. A failing provider does not roll back the others.
4. As the LAST action, pin the base currency to `1`. A provider that echoed the base in its
   response would otherwise shift the reference point for every rate.
5. Exit non-zero if any provider failed or is unknown to this build, so the scheduler sees a
   failed run and retries it. A swallowed error would leave yesterday's numbers looking fresh.

## Rules every rate adapter follows

- **An unknown code is omitted, not guessed.** Returning `1` would silently equate the
  currency with the base.
- **The base currency is not returned.** The sync pins it last.
- **Errors propagate.** A failed scheduled run is retried. A swallowed error is invisible.

See [Rate Providers](Rate-Providers).

## Failures

The reason travels as a stable code in the `x-error-code` metadata key. See
[Error Codes](Error-Codes). A stale rate (older than 24 hours) is **served** with a `WARN` log
that carries its age. Refusing would fail a payment because a third-party API is down.

## Observability

- **Logs:** one JSON object per line with the fields `ts`, `level`, `service`, `env`, `msg`
  and `logger`, plus span and event fields flattened into the line. The caller's
  `x-request-id` appears as `traceId`. See [API](API#logs).
- **Metrics:** Prometheus on a dedicated port, with names and buckets compatible with
  Micrometer-instrumented JVM services. See [API](API#metrics).
