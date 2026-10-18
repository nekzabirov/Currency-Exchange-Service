# Deployment

## Image

The repository's `Dockerfile` is a self-contained multi-stage build:

```bash
docker build -t currency-exchange-service .
```

- Build stage: `rust:1.88-slim-bookworm` plus `protobuf-compiler`, running
  `cargo build --release --locked --bins`. The release profile uses fat LTO and a single
  codegen unit, so the first build takes a few minutes.
- Runtime stage: `debian:bookworm-slim` with `ca-certificates`, running as the non-root user
  `currency` (uid `10001`).
- Binaries: `/usr/local/bin/currency-manager` (the default `CMD`), `/usr/local/bin/migrate`,
  `/usr/local/bin/seed` and `/usr/local/bin/sync`.
- Exposed ports: `9090` (gRPC), `8080` (`/__health`) and `9464` (`/metrics`).

One image serves all four roles. Pick the role with the container command.

## Roles and ordering

| Order | Role | Command | Kind | Notes |
|-------|------|---------|------|-------|
| 1 | schema | `migrate` | one-off job per release | Must succeed **before** the server rolls out. Uses a Postgres advisory lock, so concurrent runs are safe. An applied migration whose file has changed fails the run. |
| 2 | registry | `seed` | one-off job per release, after `migrate` | Insert-only, so it is safe to run on every deploy. It never touches synced rates. |
| 3 | server | `currency-manager` (default) | long-running, horizontally scalable | Loads the registry before opening its ports. |
| — | rates | `sync` | scheduled, hourly | Exits non-zero if any provider fails. Let the scheduler retry it. |

All roles need `DATABASE_URL`. See [Configuration](Configuration) for the rest.

## Kubernetes notes

- **Liveness/readiness:** use an `httpGet` probe on `/__health`, port `8080`. Do not use a
  `grpc` probe, because the server does not implement `grpc.health.v1.Health`.
- **Shutdown:** on `SIGTERM` the server stops accepting calls and drains in-flight ones. The
  metrics listener keeps answering until the process exits.
- **Ports:** `GRPC_PORT`, `HTTP_PORT` and `METRICS_PORT` are bare numbers. Derive the container
  port and the Service port from the same value.
- **Scraping:** expose `9464` to Prometheus only. It does not need ingress.
- **Secrets:** `OPENEXCHANGE_APP_ID` and `COINGECKO_API_KEY` are typically mounted from a
  secret. An empty secret key counts as unset.
- **Rates schedule:** a `CronJob` running `sync` hourly, with `concurrencyPolicy: Forbid`,
  works well. Rates older than 24 hours are still served but log `rate.stale`.
- **Database load:** each server instance uses at most 2 connections. Conversions never touch
  the database.

Example CronJob container spec:

```yaml
containers:
  - name: sync
    image: currency-exchange-service:latest
    command: ["sync"]
    env:
      - name: DATABASE_URL
        valueFrom: {secretKeyRef: {name: currency-exchange-service, key: database-url}}
      - name: OPENEXCHANGE_APP_ID
        valueFrom: {secretKeyRef: {name: currency-exchange-service, key: openexchange-app-id}}
```

## Docker Compose

`docker-compose.yml` in the repository runs the whole stack locally. See
[Getting Started](Getting-Started).

## Upgrades

- Migrations are append-only. Never edit a migration that has been applied anywhere: sqlx
  checksums every file, and the `migrate` job would fail.
- The server reads registry columns by name, so a migration that adds a column does not break
  running instances.
