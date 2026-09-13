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

Config ausschließlich aus Env (`API_ADDR`, `DATABASE_URL`, `REDIS_URL`; optional `API_READY_TIMEOUT_MS`, `API_DB_MAX_CONNECTIONS`, `API_COOKIE_SECURE`, `API_SESSION_TTL_HOURS`, `RUST_LOG`). Fehlt eine Pflichtvariable, startet der Prozess nicht und sagt welche. Beim Start laufen die sqlx-Migrationen aus `api/migrations` (Compose startet die API erst, wenn Postgres healthy ist).

| Route | Antwort |
|---|---|
| `GET /health` | `200 {"status":"ok"}` — ohne Abhängigkeiten |
| `GET /ready` | `200 {"status":"ready", ...}` nur wenn Postgres **und** Redis antworten; sonst `503 {"status":"not_ready", ...}` |
| `GET /api/auth/session` | `200 {"user": User\|null, "csrf_token"}` — Bootstrap für den Client, setzt das CSRF-Cookie falls es fehlt |
| `POST /api/auth/register` | `{email, password, name}` → `201 {"user", "csrf_token"}` + Session-Cookie; `409 email_taken`, `422 validation_failed` |
| `POST /api/auth/login` | `{email, password}` → `200 {"user", "csrf_token"}` + Session-Cookie; `401 invalid_credentials` |
| `POST /api/auth/logout` | löscht die Session, leert das Cookie → `200 {"csrf_token"}` |
| `GET /api/me` | `200 User` oder `401 unauthenticated` |
| `PATCH /api/me` | `{name?, avatar_url?}` (`avatar_url: ""` entfernt das Bild) → `200 User` |

### Auth (issue 3)

- **E-Mail + Passwort**, Hash **Argon2id** (19 MiB, t=2, p=1, PHC-String). Unbekannte E-Mail wird gegen einen beim Start vorberechneten Dummy-Hash geprüft, damit Login-Zeiten nichts verraten. Passwörter länger als 128 Zeichen lehnt auch der Login sofort ab (`invalid_credentials`).
- **Session** = Cookie `gelabber_session` (`HttpOnly; SameSite=Lax; Path=/`, `Max-Age` aus `API_SESSION_TTL_HOURS`, Default 30 Tage; `Secure` per `API_COOKIE_SECURE=true`). In Postgres liegt nur der SHA-256 des Tokens. Login/Register widerrufen die Session hinter dem mitgeschickten Cookie und räumen abgelaufene Zeilen weg — pro Browser bleibt eine Zeile.
- **CSRF**: Cookie `gelabber_csrf` (ebenfalls `HttpOnly`) plus derselbe Wert im JSON-Body von `/api/auth/session`, Login, Register und Logout. Jede Mutation unter `/api` (alles außer `GET`/`HEAD`/`OPTIONS`) braucht den Header `X-CSRF-Token` mit exakt diesem Wert, sonst `403 csrf_invalid`. Requests mit `Sec-Fetch-Site: cross-site` werden unabhängig davon abgelehnt. Login/Register/Logout rotieren das Token.
- **Fehler** kommen immer als `{"error": <code>, "message": <text>, "fields"?: {<feld>: <code>}}`. Codes: `validation_failed`, `bad_request`, `unauthenticated`, `invalid_credentials`, `csrf_invalid`, `email_taken`, `internal`. Feld-Codes: `required`, `invalid`, `too_short`, `too_long`, `taken`. Interne Ursachen stehen nur im Log.
- **Nicht in v1**: OAuth, fremdes JWT, Magic Links, 2FA, SSO, Passkeys, E2E. Avatar ist in v1 eine `https://`-URL (kein `http://`, kein Mixed Content hinter TLS); Datei-Upload kommt mit dem Dateien-/MinIO-Ticket.

Tests gegen echtes Postgres: `DATABASE_URL=postgres://gelabber:gelabber@127.0.0.1:5432/gelabber cargo test -p gelabber-api` (`#[sqlx::test]` legt pro Test eine Wegwerf-Datenbank an; ohne `DATABASE_URL` schlagen die `tests/auth.rs`-Tests fehl, `/health`- und `/ready`-Tests laufen ohne).

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

## Web

`npm run dev` in `web/` proxyt `/api` nach `127.0.0.1:8080` (Vite-Proxy), damit das httpOnly-Cookie same-origin bleibt — genau wie hinter Caddy im Compose. `VITE_API_BASE_URL` ist deshalb relativ (`/api`).

Login-Flow: `GET /api/auth/session` einmal beim Start (parallel zum ersten Render), danach hält ein Zustand-Store den User und der API-Client das CSRF-Token im Speicher. Login/Register schreiben den Store **vor** der clientseitigen Navigation, Logout und Profil-Änderungen sind optimistisch — kein Full-Reload. Feld- und Formfehler erscheinen inline (Client-Regeln spiegeln `api/src/auth/validate.rs`, Server-Feld-Codes werden auf dieselben Texte gemappt). Requests haben 10 s Timeout, Buttons wechseln nur das Label — kein hängender Spinner. Routen: `/` und `/profile` verlangen einen User, `/login` und `/register` schicken angemeldete User weiter (`?redirect=` für Deep-Links). Stirbt die Session außerhalb des Tabs (Logout woanders, TTL), kippt ein `401 unauthenticated` oder ein Bootstrap mit `user: null` den Store sofort auf anonym und die Seite springt nach `/login?redirect=…`.
