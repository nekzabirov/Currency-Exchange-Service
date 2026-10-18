# ratekeeper

**Fast gRPC currency conversion and exchange-rate service in Rust.**

ratekeeper answers two questions about money:

1. **What does this amount look like in storage units?** For example, `10.50 UAH` is
   `10500000000` units. This is a change of representation and needs no exchange rate.
2. **What is this worth in another currency?** For example, units of UAH into units of USDT.
   This is an exchange at the current rate.

It keeps a small currency registry in PostgreSQL. A scheduled `sync` job refreshes the rates
from public vendors. The server answers every conversion from an in-memory snapshot, so a
request never touches the database.

## Pages

- [Getting Started](Getting-Started): run it locally with Docker Compose.
- [Configuration](Configuration): every environment variable.
- [Architecture](Architecture): crates, binaries and design decisions.
- [API](API): the gRPC contract, metadata, HTTP endpoints, logs and metrics.
- [Money Model](Money-Model): integer units, the `1e9` scale, rounding and exchange.
- [Rate Providers](Rate-Providers): OpenExchangeRates, Coinbase, CoinGecko, and how to add one.
- [Error Codes](Error-Codes): the `x-error-code` vocabulary.
- [Deployment](Deployment): images, jobs, probes and scheduling.
- [Development](Development): building, testing and contributing.

## Documentation rule

> **Documentation rule.** Every change that affects behaviour, configuration, API, build or deployment MUST
> update `README.md` and the relevant pages in `docs/wiki/` in the same commit / pull request. A change is not
> done until the docs match the code.

The wiki is generated from [`docs/wiki/`](https://github.com/nekzabirov/ratekeeper/tree/main/docs/wiki)
in the main repository. Edit it there, not in the wiki UI.

Used in production at [1638.cloud](https://1638.cloud).
