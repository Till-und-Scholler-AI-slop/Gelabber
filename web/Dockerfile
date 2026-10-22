# syntax=docker/dockerfile:1
FROM node:26.8.2-trixie AS build
WORKDIR /src
# `.npmrc` has `legacy-peer-deps=true` (TS 7 vs typescript-eslint 8.70).
# Copy it before `npm ci` — Node 26's npm otherwise fails the peer check.
COPY package.json package-lock.json .npmrc ./
RUN npm ci
COPY . .
RUN npm run build

FROM nginx:1.31.5-alpine
ARG OCI_REVISION=unknown
ARG OCI_VERSION=unknown
ARG OCI_CREATED=unknown
ARG OCI_SOURCE=unknown
LABEL org.opencontainers.image.revision=$OCI_REVISION \
      org.opencontainers.image.version=$OCI_VERSION \
      org.opencontainers.image.created=$OCI_CREATED \
      org.opencontainers.image.source=$OCI_SOURCE
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --chmod=755 docker-entrypoint.d/40-gelabber-storage-origin.sh /docker-entrypoint.d/
COPY --from=build /src/dist /usr/share/nginx/html
# The unit tests read nginx.conf but do not parse it as nginx does. A broken
# line fails the image build here instead of the container at start, without
# and with the MinIO origin the entrypoint hands over, and with proxy delivery.
RUN nginx -t \
 && MINIO_PUBLIC_ENDPOINT=https://minio.example.com /docker-entrypoint.d/40-gelabber-storage-origin.sh \
 && nginx -t \
 && ATTACHMENT_DELIVERY=proxy /docker-entrypoint.d/40-gelabber-storage-origin.sh \
 && nginx -t \
 && rm /etc/nginx/gelabber/storage-origin.map
EXPOSE 80
