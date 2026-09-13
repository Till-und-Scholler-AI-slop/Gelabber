# Gelabber

Web-App für Chat, Video-Calls und internes Streaming. Eigenes WS-Protokoll, eigenes WebRTC-Signaling, eigener SFU/Medienpfad.

**v1 verbietet** LiveKit, mediasoup-as-a-product, Daily, Agora, Twilio Video, Stream und Socket.IO als Event-Layer. Kein Chat-/Video-Produkt-SDK. Dateien über MinIO; Live-Medien über unseren SFU + TURN (TURN folgt später).

## Lokal starten

Eine Compose-Datei, ein Reverse-Proxy:

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up
```

`docker compose up` zieht das gepinnte MinIO-CE-Image `ghcr.io/till-und-scholler-ai-slop/gelabber/minio:RELEASE.2025-10-15T17-29-55Z` (publiziert aus CI). `gelabber/minio:RELEASE.2025-10-15T17-29-55Z` ist nur das lokale Alias nach einem Source-Build. api/web/media werden nur gebaut, wenn lokal noch kein Image da ist. Fehlt das Registry-Image, fällt Compose auf den Source-Build in `deploy/compose/minio` zurück (ldflags bleiben auf dem Pin, kein `FROM minio/minio`).

Das CI-Image ist `linux/amd64`. Auf arm64 (Apple Silicon) zieht der Pull das amd64-Image (Emulation oder `exec format error`); Workaround: `docker compose up --build`.

`docker compose up --build` baut alle lokalen Images neu, inklusive MinIO aus Source — das dauert Minuten und ist nur nötig, wenn sich die Dockerfiles ändern.

Dann [http://localhost](http://localhost) (Caddy, TCP :80). Postgres, Redis und MinIO hängen an den Ports aus `.env`. Ohne `.env` gelten dieselben Dev-Defaults wie in `.env.example`.

Nach dem ersten Publish auf `main` ist das GHCR-Paket **privat**. Ein Maintainer muss es einmal öffentlich machen: Organisation → Packages → `gelabber/minio` → Package settings → Change visibility → Public. Sonst fällt ein anonymer `docker compose up` auf den Source-Build zurück. Bis dahin: `echo "$GITHUB_TOKEN" | docker login ghcr.io -u USER --password-stdin`.

UDP für späteres coturn läuft **nicht** durch Caddy.

## Schnitt

| Pfad | Rolle |
|---|---|
| `api/` | Rust-API: Axum 0.8.9, Tokio 1.53.1, **sqlx 0.9.0** (Postgres, gelockt für v1), Redis-Client, Tracing als JSON (Docker: `rust:1.98.1-slim-trixie` → `debian:trixie-slim`) |
| `web/` | React + Vite (Build: `node:26.8.2-trixie`, Runtime: `nginx:1.31.5-alpine`) |
| `media/` | Medien-Stub, dieselben Rust-Images wie die API |
| `deploy/compose` | Compose-Kern: Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z` (GHCR, Source-Build als Fallback) |

Env-Beispiele: `deploy/compose/.env.example`, `api/.env.example`, `web/.env.example`, `media/.env.example`.

## API

Config ausschließlich aus Env (`API_ADDR`, `DATABASE_URL`, `REDIS_URL`; optional `API_READY_TIMEOUT_MS`, `API_DB_MAX_CONNECTIONS`, `RUST_LOG`). Fehlt eine Pflichtvariable, startet der Prozess nicht und sagt welche.

| Route | Antwort |
|---|---|
| `GET /health` | `200 {"status":"ok"}` — ohne Abhängigkeiten |
| `GET /ready` | `200 {"status":"ready", ...}` nur wenn Postgres **und** Redis antworten; sonst `503 {"status":"not_ready", ...}` |

`/ready` prüft beide Abhängigkeiten parallel, jede mit hartem Deadline (`API_READY_TIMEOUT_MS`, Default 2000 ms). Der Body nennt pro Check Status, Latenz und eine grobe Fehlerklasse (`connection_refused`, `auth_failed`, `timed_out`, sonst `unavailable`), weil `/ready` über Caddy öffentlich erreichbar ist. Die vollständige Treiber-Fehlermeldung steht nur in der `WARN`-Logzeile (`check`, `error_class`, `error`).

```json
{"status":"not_ready","checks":{"postgres":{"status":"ok","latency_ms":3},"redis":{"status":"error","error":"connection_refused","latency_ms":0}}}
```

Jede Antwort erzeugt eine JSON-Access-Log-Zeile auf `INFO` (Methode, Pfad, Status, Latenz). Ein nicht parsbares `RUST_LOG` beendet den Start wie jede andere ungültige Env-Variable.

Lokal ohne Compose:

```bash
set -a; . api/.env.example; set +a
cargo run -p gelabber-api
```

Persistenz ist auf **sqlx 0.9.0** festgelegt (kein zweites ORM, kein Query-Builder-Mix in v1).
