# Gelabber

Chat, Voice und Go Live für eine Community auf einem Rechner. Eigenes WebSocket und eigene Signalisierung; selbst gehostetes mediasoup mit offiziellem Rust- und Browser-SDK.

## Start

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up
```

Dann [http://localhost](http://localhost). Dieser Quellstand entwickelt die noch unveröffentlichte v0.4; Compose verwendet dafür den Kandidatentag `ghcr.io/till-und-scholler-ai-slop/gelabber/{api,web,media}:v0.4.0`. Das belegt keinen veröffentlichten Release. Für diesen Entwicklungsstand erzwingt `docker compose up --build` den lokalen Build; auf arm64 ist er nötig, Image-CI baut `linux/amd64`. MinIO bleibt auf `ghcr.io/till-und-scholler-ai-slop/gelabber/minio:RELEASE.2025-10-15T17-29-55Z`.

Postgres, Redis und MinIO-Konsole hängen nur an Loopback. UDP für Voice (coturn 3478 + Relay, SFU 10000–10031) geht nicht durch Caddy.

Operator-Doku (bestehendes Caddy, TURN-Port, TLS, Backup, Grafana): [deploy/README.md](deploy/README.md).

## Layout

| Pfad | Rolle |
|---|---|
| `api/` | Rust, Axum 0.8.9, sqlx 0.9.0, Tokio 1.53.1 |
| `web/` | React + Vite, mediasoup-client 3.24.1 (`node:26.8.2-trixie` → `nginx:1.31.5-alpine`) |
| `media/` | Eigener Media-Gateway, offizielle Rust-Bindung mediasoup 0.29.0 als einzige SFU-Engine |
| `deploy/compose` | Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z`, coturn 4.18.0 |

Aktuell veröffentlicht: [v0.3.1](https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/tag/v0.3.1). v0.4 ist noch nicht veröffentlicht oder ausgerollt.

## Dev ohne Compose

Vor dem Media-Build die [gepinnten nativen Buildwerkzeuge](docs/v0.4-mediasoup-migration.md#gepinnter-nativer-build) gemäß Migrationsdokument einrichten. Die dortige Anleitung verwendet eine isolierte Python-Umgebung und begrenzt die native Buildparallelität.

```bash
set -a; . api/.env.example; set +a; cargo run -p gelabber-api
set -a; . media/.env.example; set +a; cargo run -p gelabber-media
cd web && npm run dev
```

`npm run dev` proxyt `/api` und `/ws` nach `:8080`, `/media` nach `:8081`. Tests: `DATABASE_URL=postgres://gelabber:gelabber@127.0.0.1:5432/gelabber cargo test -p gelabber-api` (Gateway zusätzlich `REDIS_URL`).

## Call-Sounds

Kurze Signaltöne melden Beitritt und Verlassen im eigenen Call sowie das eigene
Stummschalten und Taubstellen. Unter **Einstellungen → Audio** oder
**Benachrichtigungen** lassen sie sich abschalten, in der Lautstärke anpassen und
vorhören. Die Auswahl gilt pro Browser. Teilnehmer-Töne bleiben beim Taubstellen
stumm; Wiederholungen beim Verbinden lösen keine Beitrittstöne aus.

Lokale Browserprüfung ohne API oder Testkonten: Vite starten und im `web`-Ordner
`npm run test:call-sounds-smoke` ausführen. Prüft native Wiedergabe in Chromium und
Firefox ohne Autoplay-Ausnahme. Für Brave optional `BRAVE_PATH=/pfad/zu/brave` setzen.
