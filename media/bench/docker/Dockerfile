# Build context contains only frozen, Debian-loader-verified probe binaries.
FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a AS current
ARG REVISION
ARG BINARY_SHA256
LABEL org.opencontainers.image.revision=$REVISION gelabber.bench.binary_sha256=$BINARY_SHA256
COPY gelabber-current-probe /usr/local/bin/gelabber-current-probe
USER nobody
ENTRYPOINT ["/usr/local/bin/gelabber-current-probe"]

FROM debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a AS mediasoup
ARG REVISION
ARG BINARY_SHA256
LABEL org.opencontainers.image.revision=$REVISION gelabber.bench.binary_sha256=$BINARY_SHA256
COPY gelabber-mediasoup-probe /usr/local/bin/gelabber-mediasoup-probe
USER nobody
ENTRYPOINT ["/usr/local/bin/gelabber-mediasoup-probe"]
