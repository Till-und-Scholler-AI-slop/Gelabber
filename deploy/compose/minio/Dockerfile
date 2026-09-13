# syntax=docker/dockerfile:1
# MinIO CE is source-only. Compile the pinned upstream tag, then apply the
# same binary + entrypoint layout as minio/minio's Dockerfile at that tag.
# Do not FROM minio/minio — Hub images are no longer the CE distribution.

ARG MINIO_RELEASE=RELEASE.2025-10-15T17-29-55Z

FROM golang:1.24.8-alpine AS compile
RUN apk add --no-cache git ca-certificates
WORKDIR /src
ARG MINIO_RELEASE
RUN git clone --depth 1 --branch "${MINIO_RELEASE}" https://github.com/minio/minio.git .
ENV CGO_ENABLED=0
ENV MINIO_RELEASE=RELEASE
RUN LDFLAGS="$(go run buildscripts/gen-ldflags.go)" \
    && go build -tags kqueue -trimpath -ldflags "$LDFLAGS" -o /out/minio . \
    && cp dockerscripts/docker-entrypoint.sh /out/docker-entrypoint.sh

FROM alpine:3.24
RUN apk add --no-cache ca-certificates curl
COPY --from=compile /out/minio /usr/bin/minio
COPY --from=compile /out/docker-entrypoint.sh /usr/bin/docker-entrypoint.sh
RUN chmod +x /usr/bin/minio /usr/bin/docker-entrypoint.sh \
    && chmod -R 777 /usr/bin
ENTRYPOINT ["/usr/bin/docker-entrypoint.sh"]
VOLUME ["/data"]
EXPOSE 9000 9001
CMD ["minio"]
