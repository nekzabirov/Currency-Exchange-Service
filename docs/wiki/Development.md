# Development

## Requirements

- **Rust 1.88.** It is pinned in `rust-toolchain.toml`, so rustup installs it, together with
  `rustfmt` and `clippy`, on the first `cargo` call.
- **protoc.** `crates/api/build.rs` compiles the gRPC contract. Install it with
  `apt install protobuf-compiler` or `brew install protobuf`.
- **Docker** (optional), for Postgres and the full stack.

## Build and test

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs exactly these checks (`.github/workflows/ci.yml`). The tests need neither a database
nor external network access. Adapter tests serve canned HTTP responses from a local port.

## Running locally

```bash
docker compose up -d postgres
export DATABASE_URL=postgres://currency:currency@localhost:55432/currency

cargo run --bin migrate
cargo run --bin seed
cargo run --bin currency-manager      # gRPC :9090, /__health :8080, /metrics :9464

# in another shell
OPENEXCHANGE_APP_ID=... cargo run --bin sync
```

## Layout and boundaries

See [Architecture](Architecture). The boundaries that matter when you change code:

- `core` has no I/O. All money arithmetic lives in `crates/core/src/money.rs`, under tests.
- Adapters depend only on `provider-sdk` and never on `store`.
- `providers/registry` is the only place adapters are named.
- A request handler in `api` never awaits the database.

## Conventions

- Comments explain **why**, not what. Doc comments state the invariant and the price of
  breaking it.
- Errors bubble up. Unknown codes are omitted, never guessed. Money is never `f64` internally.
- Tests are named as sentences (`an_unknown_code_is_omitted_rather_than_guessed`).
- Log lines carry a stable `event` name. Never put secrets or query strings in logs (use
  `kit::redact`).
- New migrations get a new numbered file in `migrations/`. Never edit an existing one.

## Changing the contract

`proto/currency/v1/currency.proto` is a public contract. Do not rename or renumber fields,
messages or the service. Add new fields with new numbers.

## Documentation rule

> **Documentation rule.** Every change that affects behaviour, configuration, API, build or deployment MUST
> update `README.md` and the relevant pages in `docs/wiki/` in the same commit / pull request. A change is not
> done until the docs match the code.

See also [CONTRIBUTING.md](https://github.com/nekzabirov/Currency-Exchange-Service/blob/main/CONTRIBUTING.md).
