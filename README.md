# Gelabber

Web-App für Chat, Video-Calls und internes Streaming. Eigenes WS-Protokoll, eigenes WebRTC-Signaling, eigener SFU/Medienpfad.

**v1 verbietet** LiveKit, mediasoup-as-a-product, Daily, Agora, Twilio Video, Stream und Socket.IO als Event-Layer. Kein Chat-/Video-Produkt-SDK. Dateien über MinIO; Live-Medien über unseren SFU + TURN (TURN folgt später).

## Lokal starten

Eine Compose-Datei, ein Reverse-Proxy:

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up --build
```

Der **erste** `docker compose up --build` dauert Minuten: MinIO CE wird vom gepinnten Tag `RELEASE.2025-10-15T17-29-55Z` aus Source gebaut, dazu kommen die Rust- und Node-Images. Danach ist `docker compose up` (ohne `--build`) der Sub-Minuten-Pfad. Image aus CI pullen statt lokal bauen: [#19](https://github.com/Till-und-Scholler-AI-slop/Gelabber/issues/19).

Dann [http://localhost](http://localhost) (Caddy, TCP :80). Postgres, Redis und MinIO hängen an den Ports aus `.env`. Ohne `.env` gelten dieselben Dev-Defaults wie in `.env.example`.

UDP für späteres coturn läuft **nicht** durch Caddy.

## Schnitt

| Pfad | Rolle |
|---|---|
| `api/` | Rust-API: Axum 0.8.9, Tokio 1.53.1, **sqlx 0.9.0** (Postgres, gelockt für v1), Redis-Client, Tracing als JSON (Docker: `rust:1.98.1-slim-trixie` → `debian:trixie-slim`) |
| `web/` | React + Vite (Build: `node:26.8.2-trixie`, Runtime: `nginx:1.31.5-alpine`) |
| `media/` | Medien-Stub, dieselben Rust-Images wie die API |
| `deploy/compose` | Compose-Kern: Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z` (Source-Build) |

Env-Beispiele: `deploy/compose/.env.example`, `api/.env.example`, `web/.env.example`, `media/.env.example`.

## API

Config ausschließlich aus Env (`API_ADDR`, `DATABASE_URL`, `REDIS_URL`; optional `API_READY_TIMEOUT_MS`, `API_DB_MAX_CONNECTIONS`, `RUST_LOG`). Fehlt eine Pflichtvariable, startet der Prozess nicht und sagt welche.

| Route | Antwort |
|---|---|
| `GET /health` | `200 {"status":"ok"}` — ohne Abhängigkeiten |
| `GET /ready` | `200 {"status":"ready", ...}` nur wenn Postgres **und** Redis antworten; sonst `503 {"status":"not_ready", ...}` |

`/ready` prüft beide Abhängigkeiten parallel, jede mit hartem Deadline (`API_READY_TIMEOUT_MS`, Default 2000 ms), und nennt pro Check Status, Latenz und die konkrete Ursache:

```json
{"status":"not_ready","checks":{"postgres":{"status":"ok","latency_ms":3},"redis":{"status":"error","error":"Connection refused (os error 111)","latency_ms":0}}}
```

Lokal ohne Compose:

```bash
set -a; . api/.env.example; set +a
cargo run -p gelabber-api
```

Persistenz ist auf **sqlx 0.9.0** festgelegt (kein zweites ORM, kein Query-Builder-Mix in v1).
