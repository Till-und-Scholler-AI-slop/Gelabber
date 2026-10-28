# Gelabber

**Selbst hosten, frei reden.**

[![CI](https://github.com/Till-und-Scholler-AI-slop/Gelabber/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Till-und-Scholler-AI-slop/Gelabber/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Till-und-Scholler-AI-slop/Gelabber)](https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest)

Chat, Sprache und Live-Screenshare für eine Community auf dem eigenen Server.
Wer live geht, zeigt allen im Sprachkanal einen Bildschirm oder ein Fenster — auch mehrere gleichzeitig.
Eigene WebSocket-Signalisierung, selbst gehostetes mediasoup mit offiziellem Rust- und Browser-SDK.

![Gelabber Übersicht](docs/assets/readme-hero.png)

<p align="center">
  <img src="docs/assets/readme-chat.png" width="48%" alt="Textkanal" />
  <img src="docs/assets/readme-voice.png" width="48%" alt="Sprachraum" />
</p>

## Start

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up
```

Dann [http://localhost](http://localhost). App-Images: `ghcr.io/till-und-scholler-ai-slop/gelabber/{api,web,media}:v0.8.2`. Fehlt das Paket (GHCR oft privat), baut Compose aus dem Source. `docker compose up --build` erzwingt den Build; auf arm64 ist er nötig, Image-CI baut `linux/amd64`. MinIO bleibt auf `ghcr.io/till-und-scholler-ai-slop/gelabber/minio:RELEASE.2025-10-15T17-29-55Z`.

Postgres, Redis und MinIO-Konsole hängen nur an Loopback. UDP für Voice (coturn 3478 + Relay, SFU 10000 plus ein Port je weiterem Medien-Worker, siehe `deploy/README.md`) geht nicht durch Caddy.

Operator-Doku (bestehendes Caddy, TURN-Port, TLS, Backup, Grafana): [deploy/README.md](deploy/README.md).

PWA auf dem Handy: [Gelabber als PWA installieren](docs/v0.6-pwa.md).

## Layout

| Pfad | Rolle |
|---|---|
| `api/` | Rust, Axum 0.8.9, sqlx 0.9.0, Tokio 1.53.1 |
| `web/` | React + Vite, mediasoup-client 3.24.1 (`node:26.8.2-trixie` → `nginx:1.31.5-alpine`) |
| `media/` | Eigener Media-Gateway, offizielle Rust-Bindung mediasoup 0.29.0 als einzige SFU-Engine |
| `deploy/compose` | Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z`, coturn 4.18.0 |

Aktuell veröffentlicht: [v0.8.2](https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/tag/v0.8.2).

## Dev ohne Compose

Vor dem Media-Build die [gepinnten nativen Buildwerkzeuge](docs/v0.4-mediasoup-migration.md#gepinnter-nativer-build) gemäß Migrationsdokument einrichten. Die dortige Anleitung verwendet eine isolierte Python-Umgebung und begrenzt die native Buildparallelität.

```bash
set -a; . api/.env.example; set +a; cargo run -p gelabber-api
set -a; . media/.env.example; set +a; cargo run -p gelabber-media
cd web && npm run dev
```

`npm run dev` proxyt `/api` und `/ws` nach `:8080`, `/media` nach `:8081`. Tests: `DATABASE_URL=postgres://gelabber:gelabber@127.0.0.1:5432/gelabber cargo test -p gelabber-api` (Gateway zusätzlich `REDIS_URL`).

## Call-Sounds

Kurze Töne für Beitritt, Verlassen, Stummschalten und Taubstellen. Unter **Einstellungen → Audio** lassen sie sich abschalten, anpassen und vorhören. Browserprüfung ohne laufende API: `cd web && npm run test:call-sounds-smoke`.
