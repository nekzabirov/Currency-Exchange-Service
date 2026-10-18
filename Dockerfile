# syntax=docker/dockerfile:1
#
# Self-contained multi-stage build: compiles the four binaries from source and ships them on a
# slim Debian runtime.
#
#   docker build -t currency-exchange-service .
#
# Why glibc rather than alpine: rustls and reqwest link against glibc, and musl builds of this kind
# of dependency tree have proven fragile. Saving twenty megabytes is not worth it.

FROM rust:1.88-slim-bookworm AS build

# protoc is required: the gRPC contract is compiled by crates/api/build.rs, and prost does not
# bundle it.
RUN apt-get update \
 && apt-get install -y --no-install-recommends protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

# The cache mounts keep the registry and the target directory between builds; the binaries are
# copied out of the cached target directory in the same step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bins \
 && mkdir -p /out \
 && cp target/release/currency-manager target/release/migrate target/release/seed target/release/sync /out/

FROM debian:bookworm-slim

# ca-certificates are mandatory: the rate adapters call vendors over https, and without root
# certificates the TLS handshake fails with an error that does not reveal the cause.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --create-home currency

COPY --from=build /out/currency-manager /usr/local/bin/currency-manager
COPY --from=build /out/migrate          /usr/local/bin/migrate
COPY --from=build /out/seed             /usr/local/bin/seed
COPY --from=build /out/sync             /usr/local/bin/sync

USER currency

# 9090: gRPC, 8080: liveness probe (/__health), 9464: Prometheus /metrics.
EXPOSE 9090 8080 9464

CMD ["currency-manager"]
