# syntax=docker/dockerfile:1
FROM rust:1.98.1-slim-trixie AS build
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY api ./api
COPY media ./media
RUN cargo build --release -p gelabber-media

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
