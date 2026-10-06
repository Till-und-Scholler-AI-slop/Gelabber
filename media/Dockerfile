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
    CARGO_NET_RETRY=3
# The SDK's pinned planus code generator invokes rustfmt during compilation.
RUN rustup component add --toolchain 1.98.1 rustfmt && rustfmt --version
COPY docker/mediasoup-build-tools.py docker/mediasoup-build-tools.lock.json \
    docker/mediasoup-debian-build-tools.lock.tsv docker/install-mediasoup-debian-build-tools.sh \
    /opt/gelabber-native-bootstrap/
# Python and compiler packages are selected from Debian 13; native Python
# wheels are hash-verified, then all upstream pip calls use only that wheelhouse.
RUN sh /opt/gelabber-native-bootstrap/install-mediasoup-debian-build-tools.sh \
       /opt/gelabber-native-bootstrap/mediasoup-debian-build-tools.lock.tsv \
    && python3 /opt/gelabber-native-bootstrap/mediasoup-build-tools.py --destination /opt/gelabber-native-build \
    && rm -rf /var/lib/apt/lists/*
ENV PYTHON=/opt/gelabber-native-build/native-python \
    CARGO_BUILD_JOBS=2 \
    PIP_NO_INDEX=1 \
    PIP_FIND_LINKS=/opt/gelabber-native-build/wheelhouse \
    PIP_CONSTRAINT=/opt/gelabber-native-build/constraints.txt \
    MESON_VERSION=1.12.0 \
    NINJA_VERSION=1.13.2
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
RUN cargo build --locked --release -p gelabber-media \
    && cargo clean --locked --release -p gelabber-api -p gelabber-media -p gelabber-shared

FROM dependencies AS build
# Restore all original manifests/versions; no placeholder artifact may survive.
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY api/Cargo.toml ./api/Cargo.toml
COPY media ./media
COPY shared ./shared
RUN cargo build --locked --release -p gelabber-media

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
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/gelabber-media /usr/local/bin/gelabber-media
EXPOSE 8081
# Host ICE UDP (published by Compose, not Caddy).
EXPOSE 10000/udp
USER nobody
CMD ["gelabber-media"]
