# syntax=docker/dockerfile:1
FROM rust:1.98.1-slim-trixie AS build
COPY docker/rust-build-ca.sh /tmp/rust-build-ca.sh
RUN sh /tmp/rust-build-ca.sh
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    SSL_CERT_DIR=/etc/ssl/certs \
    CURL_CA_BUNDLE=/etc/ssl/certs/ca-certificates.crt \
    CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-certificates.crt \
    CARGO_NET_RETRY=3
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY api ./api
COPY media ./media
RUN cargo build --locked --release -p gelabber-media

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/gelabber-media /usr/local/bin/gelabber-media
EXPOSE 8081
# Host ICE UDP (published by Compose, not Caddy).
EXPOSE 10000-10031/udp
USER nobody
CMD ["gelabber-media"]
