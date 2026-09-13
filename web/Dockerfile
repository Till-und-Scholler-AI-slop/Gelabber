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
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/dist /usr/share/nginx/html
EXPOSE 80
