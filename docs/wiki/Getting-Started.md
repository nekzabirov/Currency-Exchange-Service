# Getting Started

## With Docker Compose

Requirements: Docker with Compose v2. You do not need a Rust toolchain: the image builds
from source.

```bash
git clone https://github.com/nekzabirov/Currency-Exchange-Service.git
cd Currency-Exchange-Service
docker compose up --build
```

Compose runs these in order:

1. `postgres` (PostgreSQL 16), published on host port `55432`.
2. `migrate` applies the schema and exits.
3. `seed` inserts the registry rows and exits. It only inserts and never overwrites.
4. `currency-manager` starts the server.

| Host port | Container port | Purpose                     |
|-----------|----------------|-----------------------------|
| `59090`   | `9090`         | gRPC                        |
| `58080`   | `8080`         | HTTP liveness `/__health`   |
| `59464`   | `9464`         | Prometheus `/metrics`       |

The seed registers five currencies, all with a scale of `1e9`:

| Code   | Kind     | Provider       |
|--------|----------|----------------|
| `EUR`  | `FIAT`   | `openexchange` |
| `UAH`  | `FIAT`   | `openexchange` |
| `USDT` | `CRYPTO` | `coinbase`     |
| `BTC`  | `CRYPTO` | `coinbase`     |
| `USDC` | `CRYPTO` | `coinbase`     |

## First calls

```bash
curl localhost:58080/__health
# ok

grpcurl -plaintext -import-path proto -proto currency/v1/currency.proto \
  -d '{"amount":10.50,"currency":"UAH"}' \
  localhost:59090 currency.v1.CurrencyService/ConvertToUnits
# {"units":"10500000000"}

grpcurl -plaintext -import-path proto -proto currency/v1/currency.proto \
  -d '{"units":"10500000000","currency":"UAH"}' \
  localhost:59090 currency.v1.CurrencyService/ConvertFromUnits
# {"amount":10.5}
```

The server does not enable gRPC reflection, so pass the `.proto` file to `grpcurl` as shown.

## Loading exchange rates

Straight after seeding, no currency has a rate yet. `ConvertToUnits` and `ConvertFromUnits`
still work, because they do not use rates. `ConvertUnits` refuses with `RATE_MISSING` until
the first sync:

```bash
OPENEXCHANGE_APP_ID=<your-app-id> docker compose run --rm sync
```

You can get a free app id at [openexchangerates.org](https://openexchangerates.org/). Without
it, the fiat adapter refuses and the run exits non-zero. Coinbase rates for `USDT`, `BTC` and
`USDC` are still written, because Coinbase needs no key.

The server picks up new rates within 60 seconds, which is its refresh interval. Then:

```bash
grpcurl -plaintext -import-path proto -proto currency/v1/currency.proto \
  -d '{"units":"1000000000","currency":"EUR","to_currency":"USDT"}' \
  localhost:59090 currency.v1.CurrencyService/ConvertUnits
```

In production, run `sync` on a schedule (hourly is the intended cadence). See
[Deployment](Deployment).

## Without Docker

See [Development](Development) to build and run the binaries directly with Cargo.
