# Gelabber

Chat, Voice und Go Live für eine Community auf einem Rechner. Eigenes WebSocket, eigener SFU. Kein LiveKit, kein fremdes Chat-/Video-SDK.

## Start

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up
```

Dann [http://localhost](http://localhost). Images: `ghcr.io/till-und-scholler-ai-slop/gelabber/{api,web,media,minio}:v0.2.3`. Fehlt das Paket (GHCR oft privat), baut Compose aus dem Source. `docker compose up --build` erzwingt den Build — auf arm64 nötig, CI ist `linux/amd64`.

Postgres, Redis und MinIO-Konsole hängen nur an Loopback. UDP für Voice (coturn 3478 + Relay, SFU 10000–10031) geht nicht durch Caddy.

Operator-Doku (bestehendes Caddy, TURN-Port, TLS, Backup, Grafana): [deploy/README.md](deploy/README.md).

## Layout

| Pfad | Rolle |
|---|---|
| `api/` | Rust, Axum 0.8.9, sqlx 0.9.0, Tokio 1.53.1 |
| `web/` | React + Vite (`node:26.8.2-trixie` → `nginx:1.31.5-alpine`) |
| `media/` | SFU, webrtc 0.20.5 |
| `deploy/compose` | Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z`, coturn 4.18.0 |

Release: [v0.2.3](https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/tag/v0.2.3).

## Dev ohne Compose

```bash
set -a; . api/.env.example; set +a; cargo run -p gelabber-api
set -a; . media/.env.example; set +a; cargo run -p gelabber-media
cd web && npm run dev
```

`npm run dev` proxyt `/api` und `/ws` nach `:8080`, `/media` nach `:8081`. Tests: `DATABASE_URL=postgres://gelabber:gelabber@127.0.0.1:5432/gelabber cargo test -p gelabber-api` (Gateway zusätzlich `REDIS_URL`).
