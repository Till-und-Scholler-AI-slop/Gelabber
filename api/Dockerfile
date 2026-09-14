# syntax=docker/dockerfile:1
FROM rust:1.98.1-slim-trixie AS build
# rustup's cargo uses a vendored libcurl that does not always load Debian's
# CA bundle. crates.io then fails with OpenSSL verify 19 (self-signed
# certificate in certificate chain) during `docker compose up`.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && update-ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
    SSL_CERT_DIR=/etc/ssl/certs \
    CURL_CA_BUNDLE=/etc/ssl/certs/ca-certificates.crt \
    CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-certificates.crt \
    CARGO_NET_RETRY=3
# rustup sets CARGO_HOME=/usr/local/cargo; config.toml is the file cargo
# always reads (ENV alone is not enough for some vendored libcurl builds).
RUN printf '%s\n' \
      '[http]' \
      'cainfo = "/etc/ssl/certs/ca-certificates.crt"' \
      'multiplexing = false' \
      '[net]' \
      'retry = 3' \
      > /usr/local/cargo/config.toml
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY api ./api
COPY media ./media
RUN cargo build --locked --release -p gelabber-api

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/gelabber-api /usr/local/bin/gelabber-api
EXPOSE 8080
USER nobody
CMD ["gelabber-api"]
