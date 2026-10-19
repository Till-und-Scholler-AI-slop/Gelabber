# syntax=docker/dockerfile:1
# MinIO client for the one-shot minio-init service (#196). Like the server
# (../minio), built from the pinned upstream tag; Hub images are no longer
# the CE distribution. The init script is mounted, not baked in, so the
# pinned image never needs a rebuild for script changes.
# CI publishes the pin to GHCR (.github/workflows/publish-minio.yml).

ARG MC_RELEASE=RELEASE.2025-08-13T08-35-41Z

FROM golang:1.24.8-alpine AS compile
RUN apk add --no-cache git ca-certificates
WORKDIR /src
ARG MC_RELEASE
RUN git clone --depth 1 --branch "${MC_RELEASE}" https://github.com/minio/mc.git .
ENV CGO_ENABLED=0
RUN VERSION="${MC_RELEASE#RELEASE.}" \
    && LDFLAGS="$(MC_RELEASE=RELEASE go run buildscripts/gen-ldflags.go "${VERSION}")" \
    && go build -tags kqueue -trimpath -ldflags "$LDFLAGS" -o /out/mc .

FROM alpine:3.24
RUN apk add --no-cache ca-certificates
COPY --from=compile /out/mc /usr/bin/mc
ENTRYPOINT ["/usr/bin/mc"]
