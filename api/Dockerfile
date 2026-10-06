# syntax=docker/dockerfile:1
FROM rust:1.98.1-slim-trixie@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS toolchain

FROM toolchain AS base
COPY docker/debian-snapshot.sources /etc/apt/sources.list.d/debian.sources
COPY docker/rust-build-ca.sh /tmp/rust-build-ca.sh
RUN sh /tmp/rust-build-ca.sh
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    SSL_CERT_DIR=/etc/ssl/certs \
    CURL_CA_BUNDLE=/etc/ssl/certs/ca-certificates.crt \
    CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-certificates.crt \
    CARGO_NET_RETRY=3 \
    CARGO_BUILD_JOBS=2
FROM base AS recipe
WORKDIR /input
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY api/Cargo.toml ./api/Cargo.toml
COPY media/Cargo.toml ./media/Cargo.toml
COPY shared/Cargo.toml ./shared/Cargo.toml
COPY docker/rust-deps-recipe.sh /tmp/rust-deps-recipe.sh
RUN sh /tmp/rust-deps-recipe.sh /input /recipe

FROM base AS dependencies
WORKDIR /src
# The recipe contents stay identical across source-only and version-only changes.
COPY --from=recipe /recipe/ ./
RUN cargo build --locked --release -p gelabber-api \
    && cargo clean --locked --release -p gelabber-api -p gelabber-media -p gelabber-shared

FROM dependencies AS build
# Restore all original manifests/versions; no placeholder artifact may survive.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY media/Cargo.toml ./media/Cargo.toml
COPY api ./api
COPY shared ./shared
RUN cargo build --locked --release -p gelabber-api

FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
# Bootstrap HTTPS APT with the untouched, digest-pinned public CA bundle.
# Build-time proxy certificates must never be copied into the runtime image.
COPY --from=toolchain /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY docker/debian-snapshot.sources /etc/apt/sources.list.d/debian.sources
ARG OCI_REVISION=unknown
ARG OCI_VERSION=unknown
ARG OCI_CREATED=unknown
ARG OCI_SOURCE=unknown
LABEL org.opencontainers.image.revision=$OCI_REVISION \
      org.opencontainers.image.version=$OCI_VERSION \
      org.opencontainers.image.created=$OCI_CREATED \
      org.opencontainers.image.source=$OCI_SOURCE
RUN apt-get -o Acquire::https::CaInfo=/etc/ssl/certs/ca-certificates.crt update \
    && apt-get -o Acquire::https::CaInfo=/etc/ssl/certs/ca-certificates.crt install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/gelabber-api /usr/local/bin/gelabber-api
EXPOSE 8080
USER nobody
CMD ["gelabber-api"]
