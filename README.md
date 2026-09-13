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

Dann [http://localhost](http://localhost) (Caddy, TCP :80). Postgres, Redis und MinIO hängen an den Ports aus `.env`.

UDP für späteres coturn läuft **nicht** durch Caddy.

## Schnitt

| Pfad | Rolle |
|---|---|
| `api/` | Rust-API (Docker: `rust:1.98.1-slim-trixie` → `debian:trixie-slim`) |
| `web/` | React + Vite (Build: `node:26.8.2-trixie`, Runtime: `nginx:1.31.5-alpine`) |
| `media/` | Medien-Stub, dieselben Rust-Images wie die API |
| `deploy/compose` | Compose-Kern: Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z` (Source-Build) |

Env-Beispiele: `deploy/compose/.env.example`, `api/.env.example`, `web/.env.example`, `media/.env.example`.
