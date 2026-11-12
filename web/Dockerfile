# syntax=docker/dockerfile:1
FROM node:26.12.1-trixie AS build
WORKDIR /src
# `.npmrc` has `legacy-peer-deps=true` (TS 7 vs typescript-eslint 8.70).
# Copy it before `npm ci` — Node 26's npm otherwise fails the peer check.
COPY package.json package-lock.json .npmrc ./
RUN npm ci
COPY . .
RUN npm run build

FROM nginx:1.31.7-alpine
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
COPY --chmod=755 docker-entrypoint.d/50-gelabber-public-pages.sh /docker-entrypoint.d/
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
# The same for the public pages (#522): off, as on a self-hosted instance by
# default, then on with an Impressum, which writes the landing map. Every
# start writes them anew, so the image keeps none.
RUN /docker-entrypoint.d/50-gelabber-public-pages.sh \
 && nginx -t \
 && test -f /usr/share/nginx/public-pages/public-pages.json \
 && test ! -e /etc/nginx/gelabber/public/landing.map \
 && GELABBER_LANDING=true \
    GELABBER_OPERATOR_NAME="Example Operator" \
    GELABBER_OPERATOR_STREET="Example Street 1" \
    GELABBER_OPERATOR_CITY="12345 Example City" \
    GELABBER_OPERATOR_EMAIL=operator@example.com \
    APP_PUBLIC_URL=https://gelabber.example.com \
    /docker-entrypoint.d/50-gelabber-public-pages.sh \
 && nginx -t \
 && test -f /usr/share/nginx/public-pages/de/index.html \
 && test -f /usr/share/nginx/public-pages/en/imprint.html \
 && test -f /etc/nginx/gelabber/public/landing.map \
 && rm -r /usr/share/nginx/public-pages /etc/nginx/gelabber/public
EXPOSE 80
