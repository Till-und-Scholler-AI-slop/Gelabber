# Gelabber

Web-App für Chat, Video-Calls und internes Streaming. Eigenes WS-Protokoll, eigenes WebRTC-Signaling, eigener SFU/Medienpfad.

**v1 verbietet** LiveKit, mediasoup-as-a-product, Daily, Agora, Twilio Video, Stream und Socket.IO als Event-Layer. Kein Chat-/Video-Produkt-SDK. Dateien über MinIO; Live-Medien über unseren SFU + coturn.

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

UDP für coturn (3478 + Relay) und SFU-ICE (10000–10031) läuft **nicht** durch Caddy.

## Schnitt

| Pfad | Rolle |
|---|---|
| `api/` | Rust-API: Axum 0.8.9, Tokio 1.53.1, **sqlx 0.9.0** (Postgres, gelockt für v1), Redis-Client, Tracing als JSON (Docker: `rust:1.98.1-slim-trixie` → `debian:trixie-slim`) |
| `web/` | React + Vite (Build: `node:26.8.2-trixie`, Runtime: `nginx:1.31.5-alpine`) |
| `media/` | Eigener SFU (webrtc **0.20.5**, gelockt; 0.21 ist RC): Room = Sprachkanal, RTP-Forward, kurze Join-Tickets. Dieselben Rust-Images wie die API |
| `deploy/compose` | Compose-Kern: Caddy 2.11.4, Postgres 18.6, Redis 8.10.1, MinIO CE `RELEASE.2025-10-15T17-29-55Z`, **coturn 4.18.0** (UDP nicht durch Caddy) |

Env-Beispiele: `deploy/compose/.env.example`, `api/.env.example`, `web/.env.example`, `media/.env.example`.

## API

Config ausschließlich aus Env (`API_ADDR`, `DATABASE_URL`, `REDIS_URL`; optional `API_READY_TIMEOUT_MS`, `API_DB_MAX_CONNECTIONS`, `API_COOKIE_SECURE`, `API_SESSION_TTL_HOURS`, `API_WS_HEARTBEAT_MS`, `API_WS_DEAD_MS`, `API_WS_REPLAY`, `API_WS_IDLE_MS`, `API_WS_PRESENCE_TTL_MS`, `API_WS_TYPING_TTL_MS`, `RUST_LOG`). Fehlt eine Pflichtvariable, startet der Prozess nicht und sagt welche. Beim Start laufen die sqlx-Migrationen aus `api/migrations` (Compose startet die API erst, wenn Postgres healthy ist).

| Route | Antwort |
|---|---|
| `GET /health` | `200 {"status":"ok"}` — ohne Abhängigkeiten |
| `GET /ready` | `200 {"status":"ready", ...}` nur wenn Postgres **und** Redis antworten; sonst `503 {"status":"not_ready", ...}` |
| `GET /ws` | Native WebSocket (issue 6). Session-Cookie, kein Socket.IO. `401` ohne Session; Browser-`Origin` muss zu `Host` passen. |
| `POST /api/channels/{id}/media-ticket` | Session + `join_voice` + Sprachkanal → `200 {ticket, expires_in, media_path, ice_servers}`; 12-Zeichen-Code in Redis (`gb:mt:{code}`, 30 s). Textkanal: `400`, fremd: `404` |
| `GET /api/auth/session` | `200 {"user": User\|null, "csrf_token"}` — Bootstrap für den Client, setzt das CSRF-Cookie falls es fehlt |
| `POST /api/auth/register` | `{email, password, name}` → `201 {"user", "csrf_token"}` + Session-Cookie; `409 email_taken`, `422 validation_failed` |
| `POST /api/auth/login` | `{email, password}` → `200 {"user", "csrf_token"}` + Session-Cookie; `401 invalid_credentials` |
| `POST /api/auth/logout` | löscht die Session, leert das Cookie → `200 {"csrf_token"}` |
| `GET /api/me` | `200 User` oder `401 unauthenticated` |
| `PATCH /api/me` | `{name?, avatar_url?}` (`avatar_url: ""` entfernt das Bild) → `200 User` |

### WS-Gateway (issue 6)

Dieselbe Axum-App, natives WebSocket auf `/ws` (Caddy und Vite-Proxy leiten durch). Auth ist die bestehende Session (`gelabber_session`). CSRF gilt nicht: der Handshake ist GET. Fan-out über Redis **8.10.1** Pub/Sub (`PSUBSCRIBE gb:*`); Catch-up liegt in einer begrenzten Redis-Liste pro Topic (`API_WS_REPLAY`, Default 256) — Pub/Sub selbst speichert nichts.

Kompaktes JSON, kurze Keys. Signaling (issue 10) nutzt `op:"sig"` und mischt sich nicht in den Chat-Strom (`op:"e"`).

Client → Server:

| Frame | Bedeutung |
|---|---|
| `{"op":"h"}` | Heartbeat |
| `{"op":"s","s":"<server>","c?":"<channel>","n?":<seq>}` | Subscribe; `n` = letzte gesehene Seq (Reconnect) |
| `{"op":"u","s":"<server>","c?":"<channel>"}` | Unsubscribe |
| `{"op":"p","st?":"o\|i"}` | Presence-Puls (aktiv) bzw. Idle dieses Clients |
| `{"op":"y","s":"…","c":"…","on":true\|false}` | Typing start / stop im Kanal |

Server → Client:

| Frame | Bedeutung |
|---|---|
| `{"op":"h"}` | Heartbeat (Client antwortet mit `h`) |
| `{"op":"ok","s":"…","c?":"…","n":<seq>}` | Subscribe steht; `n` ist der aktuelle Kopf |
| `{"op":"e","t":"c\|e\|d","s":"…","c?":"…","n":<seq>,"i?":"<id>","d?":{…}}` | Event: create / edit (Delta in `d`) / delete |
| `{"op":"gap","s":"…","c?":"…"}` | Lücke größer als der Replay-Puffer — History kommt per REST |
| `{"op":"err","e":"not_found\|bad_request\|…"}` | Subscribe oder Signaling abgelehnt (fremde Server/Kanäle: `not_found`, wie REST) |
| `{"op":"p","s":"…","u":"…","st":"o\|i\|x"}` | Presence-Update (online / idle / offline). Kein Seq. |
| `{"op":"p","s":"…","snap":[{"u":"…","st":"o\|i"},…]}` | Presence-Snapshot nach Subscribe |
| `{"op":"y","s":"…","c":"…","u":"…","on":true\|false}` | Typing-Broadcast. Kein Seq. |
| `{"op":"sig","t":"j\|l\|o\|a\|i\|p\|u\|m\|d\|r","s":"…","c?":"…","u?":"…",…}` | Voice-Signaling (issue 10 + 12), live, ohne Seq |

Client → Server zusätzlich:

| Frame | Bedeutung |
|---|---|
| `{"op":"sig","t":"j","s":"…","c":"…"}` | Voice-Join (Session + `join_voice` + Sprachkanal) |
| `{"op":"sig","t":"l","s":"…","c":"…"}` | Leave |
| `{"op":"sig","t":"m\|d","s":"…","c":"…","on":true\|false}` | Mute / Deafen (session-lokal; UI sofort, danach Sync) |
| `{"op":"sig","t":"o\|a","s":"…","c":"…","sdp":"…"}` | SDP offer / answer (nur im Room; Client sendet das über den Media-WS) |
| `{"op":"sig","t":"i","s":"…","c":"…","ice":"…","mid?"}` | Trickle-ICE |
| `{"op":"sig","t":"p\|u","s":"…","c":"…","k":"a\|v"}` | Pub / unpub (`v` braucht `go_live`) |

Topics: Server `gb:s:{id}` und Kanal `gb:c:{id}` sind getrennt. Signaling hängt an `gb:v:{channel}` (Pub/Sub, kein Replay). Join nur mit gültiger Session und `join_voice`; Textkanäle: `bad_request`, fremde IDs: `not_found`. Join/Leave/Mute/Deafen (`j/l/m/d`) und Pub/Unpub gehen an alle Sockets, die den **Server** subscribed haben — die Mitgliederliste zeigt Voice-State ohne im Kanal zu sitzen. SDP/ICE (`o/a/i`) bleiben im Room. Occupancy-Snapshot nach Subscribe: `{"op":"sig","t":"r","s":"…","snap":[{"u","c","m?","d?"}]}`. Redis `gb:vo:{server}` (user → Kanal + Mute/Deafen). Der Web-Client spricht natives `RTCPeerConnection` — Join-Klick und Mute/Deafen setzen den lokalen State sofort, ICE läuft im Hintergrund. Kein LiveKit, kein fremdes Medien-JWT. SDP/ICE und RTP gehen über den **media**-WS (`/media/ws`) mit kurzem internem Ticket; der Chat-WS bleibt bei Presence (`j/l/p/u/m/d`). Chat-Events nur an passende Subscribes. Heartbeat alle `API_WS_HEARTBEAT_MS` (15 s); ohne Client-Frame für `API_WS_DEAD_MS` (30 s) schließt der Server (stiller Tod). Reconnect schickt `s` mit letzter Seq — der Server spielt `n+1…` nach, ohne Doppelte. Issue 5 (REST-Nachrichten) publiziert nach einem erfolgreichen Write über `publish_channel` / `publish_server`; dieses Ticket legt keine Message-Tabellen an.

**Presence / Typing (issue 8).** Ephemeral, eigenes `op`, kein Chat-Seq. Redis-Keys mit TTL: `gb:p:c:{user}:{conn}` (Status pro Client), `gb:p:u:{user}` (Aggregat), `gb:p:s:{server}` (wer auf dem Server sichtbar ist), `gb:y:{channel}:{user}` (Typing). Fan-out über Pub/Sub `gb:p:{server}` / `gb:y:{channel}` — nicht über das Replay-Log. Idle ist **pro Client** (`API_WS_IDLE_MS`, Default 5 min); Heartbeat zählt nicht als Aktivität. User ist online, solange ein Client online ist, idle wenn alle verbliebenen idle sind, offline wenn der letzte Socket weg ist oder der Key abläuft. Typing ist in unter 100 ms sichtbar und verschwindet per Stop-Event plus Client-/Key-Timeout (`API_WS_TYPING_TTL_MS`, 6 s). Presence-Punkte leben in der Mitgliederliste, nicht in der Message-Pane — kein Relayout des Chats.

Der Web-Client hält eine Socket-Instanz pro Tab, subscribed Server/Kanal aus der URL und dedupliziert über Seq.
| `GET /api/servers` | `200 [Server]` — die Server des Users in Beitrittsreihenfolge, je mit `role`, `permissions` (effektiv) und `member_permissions` |
| `POST /api/servers` | `{name}` → `201 ServerDetail` (Owner-Mitgliedschaft, Kategorie „Textkanäle“, Kanal `#allgemein`) |
| `GET /api/servers/{id}` | `200 ServerDetail` = Server + `categories`, `channels`, `members`; fremder/unbekannter Server → `404 not_found` |
| `PATCH /api/servers/{id}` | `{name?, member_permissions?: [flag]}` → `200 Server` (`manage_server`) |
| `DELETE /api/servers/{id}` | `204` (nur Owner, kaskadiert) |
| `POST /api/servers/{id}/leave` | `204` (Member; Owner bekommt `403`) |
| `POST /api/servers/{id}/kick` | `{user_id}` → `204` (`manage_server`; nicht Owner/sich selbst) |
| `POST /api/servers/{id}/ban` | `{user_id}` → `204` (`manage_server`; sperrt Rejoin inkl. Invite) |
| `GET /api/servers/{id}/bans` | `200 [Ban]` (`manage_server`) |
| `DELETE /api/servers/{id}/bans/{user_id}` | `204` entsperrt (`manage_server`) |
| `POST /api/servers/{id}/categories` | `{name}` → `201 Category` (`manage_channels`) |
| `PATCH` / `DELETE /api/categories/{id}` | `{name}` → `200 Category` / `204`, Kanäle bleiben ohne Kategorie (`manage_channels`) |
| `POST /api/servers/{id}/channels` | `{name, kind?: "text"\|"voice", category_id?}` → `201 Channel` (`manage_channels`); Textkanal-Namen werden zu `off-topic`-Slugs; `kind: "dm"` ist ungültig |
| `PATCH` / `DELETE /api/channels/{id}` | `{name?, category_id?}` (`""` = ohne Kategorie) → `200 Channel` / `204` (`manage_channels`); DMs → `404` |
| `GET /api/dms` | `200 [DirectMessage]` — 1:1-DMs des Users, zuletzt aktiv zuerst; `peer` ist die andere Person |
| `POST /api/dms` | `{user_id}` → `201` neu / `200` bestehende DM (idempotent, Reihenfolge egal); Selbst-DM → `422 user_id invalid`; unbekannter User → `404` |
| `GET /api/dms/{id}` | `200 DirectMessage`; fremde/unbekannte DM → `404` |
| `GET /api/servers/{id}/invites` | `200 [Invite]` aktive Links (`manage_server`) |
| `POST /api/servers/{id}/invites` | `{max_uses?, expires_in_hours?}` → `201 Invite` (jedes Mitglied) |
| `GET /api/invites/{code}` | `200 {code, server: {id, name, member_count}, expires_at, member}`; `404 not_found`, `410 invite_invalid`, `403 banned` |
| `POST /api/invites/{code}/join` | `200 Server` — tritt bei (idempotent, verbraucht nur beim ersten Mal eine Nutzung); gebannt → `403 banned` |
| `DELETE /api/invites/{code}` | `204` (`manage_server` oder Ersteller) |
| `GET /api/channels/{id}/messages` | `200 {messages, has_more}` — kanal-lokale History, oldest→newest; `before`/`after` = Message-ID oder RFC3339, `limit` 1–100 (Default 50). Jede Message trägt `attachments`. Voice / fremder Kanal → `404`. DMs nutzen dieselben Pfade |
| `POST /api/channels/{id}/messages` | `{content, attachment_ids?}` — Text 0–2000 Zeichen, mindestens Text oder ein Anhang → `201 Message` (`send_messages`; in einer DM dürfen beide schreiben; Anhänge zusätzlich `send_files`) |
| `PATCH /api/messages/{id}` | `{content}` → `200 Message` (nur Autor + `send_messages`) |
| `DELETE /api/messages/{id}` | `204` (Autor oder `manage_messages`) |
| `POST /api/channels/{id}/attachments` | `{filename, content_type, size}` → `201 {id, upload_url, headers, expires_in, attachment}` Presign-PUT gegen MinIO (`send_files`; in einer DM beide) |
| `GET /api/attachments/{id}` | Auth-Check, dann Bytes bzw. 302 auf kurzlebige Presign-GET. Kein öffentliches Objekt ohne Mitgliedschaft |

### Server, Kanäle, Rechte (issue 4)

- **Struktur**: Server → Kategorien → Kanäle (`text`/`voice`). Löschen einer Kategorie lässt die Kanäle stehen (`category_id = NULL`). Keine Positionen/Reorder, keine Threads/Foren/Stages in v1.
- **Rollen grob**: Owner = `servers.owner_id`, jede andere Zeile in `server_members` ist Member. Kein Ownership-Transfer in v1.
- **Rechte-Flags** (`api/src/servers/permissions.rs`): `manage_server`, `manage_channels`, `manage_messages`, `send_messages`, `send_files`, `join_voice`, `go_live`. Pro Server **eine** Maske für alle Member (`member_permissions`, Default: alles außer verwalten); der Owner hat immer alles. Keine Channel-Overwrites. Handler prüfen mit `Membership::require(Permission::…)`. Kick/Ban brauchen `manage_server`; fremde Nachrichten löschen braucht `manage_messages`. Text-Chat hängt an `send_messages`; Dateien an `send_files`.
- **Sichtbarkeit**: Jeder Lesezugriff läuft über die Mitgliedschaft. Ein Server, in dem man nicht ist, antwortet `404 not_found` (nicht `403`), ungültige UUIDs im Pfad ebenso — die API bestätigt keine fremden IDs. Fehlende Rechte sind `403 forbidden`.
- **Einladungen**: Code = 12 Zeichen aus einem eindeutig lesbaren Klein-Alphabet (kein `0/o`, `1/l/i`), Groß-/Kleinschreibung beim Einlösen egal. Link = `/invite/{code}` in der Web-App. Beitritt läuft in einer Transaktion mit `FOR UPDATE`, damit `max_uses` auch bei gleichzeitigen Klicks hält. Abgelaufene Links fallen aus der Liste; unbekannte sind `404`, tote `410 invite_invalid`, gebannte User `403 banned`.
- Fehler-Codes zusätzlich zu Auth: `forbidden`, `not_found`, `invite_invalid`, `banned`. Feld-Codes wie bisher; neue Felder: `kind`, `category_id`, `member_permissions`, `max_uses`, `expires_in_hours`, `content`, `before`, `after`, `limit`, `user_id`, `filename`, `content_type`, `size`, `attachment_ids`.

### Direktnachrichten (issue 9)

- **Kanaltyp**: `kind = dm`, genau zwei Teilnehmer in `channel_members`. Kein Server, keine Kategorie, kein Gruppen-DM, keine Freundesliste. `pair_key` (sortierte User-IDs) macht A→B und B→A zur selben Zeile.
- **Dieselben Pfade**: History/Senden/Edit/Delete über `/api/channels/{id}/messages` und `/api/messages/{id}`. WS-Subscribe/Typing/Events sind die bestehenden Frames; bei einer DM ist `s` die Kanal-ID (`{"op":"s","s":"<dm>","c":"<dm>"}`).
- **UI**: Home-Kachel in der Server-Rail, Sidebar unter `/d` und `/d/$channelId`. Klick auf ein Mitglied (nicht sich selbst) öffnet die DM wie einen Kanal — bekannte DMs aus dem Cache sofort, erste Öffnung per idempotentem POST. Composer ist derselbe optimistic Send (`tmp:`-Zeile, < 50 ms lokal).
- **Nicht in v1**: Gruppen-DM, Message Requests, Block-Listen.

### Textnachrichten (issue 5)

- **Schreiben / Edit / Delete**: Textkanäle und 1:1-DMs. Lesen darf jedes Mitglied (bzw. beide DM-Teilnehmer); Posten und Editieren brauchen `send_messages` im Server, in einer DM dürfen beide schreiben; der Autor löscht die eigene Zeile, `manage_messages` löscht fremde auf einem Server. Delete fächert per WS (`t:"d"`) an alle Subscribes — die Zeile verschwindet sofort, die Liste springt nicht. Fremde oder Voice-Kanäle sind `404 not_found`. Inhalt: getrimmt, 1–2000 Zeichen, kein Steuerzeichen außer Tab/Newline.
- **History**: kanal-lokal, Cursor `before`/`after` als Message-ID oder RFC3339-Zeit, `limit` 1–100 (Default 50). Antwort `messages` oldest→newest plus `has_more` in der geblätterten Richtung. Keine globale Suche, keine Threads/Reaktionen.
- sqlx bleibt 0.9.0.

### Moderation (issue 15)

- **Mod-Delete**: `DELETE /api/messages/{id}` mit `manage_messages` (Owner hat das Flag immer). Gleiches Event wie Own-Delete (`op:"e","t":"d"`) — jeder Client nimmt die Zeile aus dem Cache; Scroll-Pin hält die erste sichtbare Nachricht.
- **Admin Kick/Ban**: `manage_server`. Kick entfernt die Mitgliedschaft; Ban schreibt `server_bans` und entfernt die Mitgliedschaft. Owner und der eigene Account sind keine Ziele. WS: Server-Topic `t:"d"` plus `err` `kicked`/`banned` an die Sockets des Ziels; Subscribe auf den Server ist danach `not_found`.
- **Ban blockt Rejoin**: Preview und Join über Invite antworten `403 banned`. Nach Entsperren gilt der Link wieder.

### Anhänge / MinIO (issue 7)

- **Presign**: `POST /api/channels/{id}/attachments` legt eine Metadaten-Zeile an (`message_id` noch leer) und gibt eine zeitlich begrenzte PUT-URL gegen MinIO zurück. Der Client lädt direkt; die API nimmt den Dateikörper nicht entgegen.
- **Nachricht**: `attachment_ids` (max. 1 in v1) bindet die Zeile. Der Store macht `Head` — ohne Objekt oder mit anderer Größe ist das `422`. Das WS-Create-Event trägt dasselbe `attachments`-Array wie REST.
- **Download**: `GET /api/attachments/{id}` prüft die Mitgliedschaft (Pending-Upload nur der Uploader) und streamt oder 302 auf eine kurzlebige Presign-GET. Keine öffentlichen Bucket-URLs.
- **Limits** (serverseitig): 25 MiB; `image/jpeg|png|gif|webp`, `application/pdf`, `text/plain`, `application/zip`, `audio/mpeg|wav`, `video/mp4`.
- **Feel**: Bild-Preview steht sofort (Object-URL); der Upload läuft im Hintergrund und blockiert den Composer nicht.
- **MinIO**: Image bleibt Source-Build `RELEASE.2025-10-15T17-29-55Z`. Compose setzt `MINIO_PUBLIC_ENDPOINT` (Browser) und `MINIO_API_CORS_ALLOW_ORIGIN`. sqlx bleibt 0.9.0. Kein LiveKit.

### Auth (issue 3)

- **E-Mail + Passwort**, Hash **Argon2id** (19 MiB, t=2, p=1, PHC-String). Unbekannte E-Mail wird gegen einen beim Start vorberechneten Dummy-Hash geprüft, damit Login-Zeiten nichts verraten. Passwörter länger als 128 Zeichen lehnt auch der Login sofort ab (`invalid_credentials`).
- **Session** = Cookie `gelabber_session` (`HttpOnly; SameSite=Lax; Path=/`, `Max-Age` aus `API_SESSION_TTL_HOURS`, Default 30 Tage; `Secure` per `API_COOKIE_SECURE=true`). In Postgres liegt nur der SHA-256 des Tokens. Login/Register widerrufen die Session hinter dem mitgeschickten Cookie und räumen abgelaufene Zeilen weg — pro Browser bleibt eine Zeile.
- **CSRF**: Cookie `gelabber_csrf` (ebenfalls `HttpOnly`) plus derselbe Wert im JSON-Body von `/api/auth/session`, Login, Register und Logout. Jede Mutation unter `/api` (alles außer `GET`/`HEAD`/`OPTIONS`) braucht den Header `X-CSRF-Token` mit exakt diesem Wert, sonst `403 csrf_invalid`. Requests mit `Sec-Fetch-Site: cross-site` werden unabhängig davon abgelehnt. Login/Register/Logout rotieren das Token.
- **Fehler** kommen immer als `{"error": <code>, "message": <text>, "fields"?: {<feld>: <code>}}`. Codes: `validation_failed`, `bad_request`, `unauthenticated`, `invalid_credentials`, `csrf_invalid`, `email_taken`, `forbidden`, `not_found`, `invite_invalid`, `banned`, `internal`. Feld-Codes: `required`, `invalid`, `too_short`, `too_long`, `taken`. Interne Ursachen stehen nur im Log.
- **Nicht in v1**: OAuth, fremdes JWT, Magic Links, 2FA, SSO, Passkeys, E2E. Avatar ist in v1 eine `https://`-URL (kein `http://`, kein Mixed Content hinter TLS); Datei-Upload kommt mit dem Dateien-/MinIO-Ticket.

Tests gegen echtes Postgres: `DATABASE_URL=postgres://gelabber:gelabber@127.0.0.1:5432/gelabber cargo test -p gelabber-api` (`#[sqlx::test]` legt pro Test eine Wegwerf-Datenbank an; ohne `DATABASE_URL` schlagen die `tests/auth.rs`-, `tests/servers.rs`-, `tests/messages.rs`-, `tests/dms.rs`-, `tests/moderation.rs`- und `tests/attachments.rs`-Tests fehl, `/health`- und `/ready`-Tests laufen ohne). Gateway-Tests (`tests/gateway.rs`) brauchen zusätzlich Redis (`REDIS_URL`, Default `redis://127.0.0.1:6379`). Der In-Process-HTTP-Client mit Cookie-Jar liegt in `tests/common/mod.rs`. Attachment-Bytes laufen in den Tests über den In-Memory-Store (kein MinIO).

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

```bash
set -a; . media/.env.example; set +a
cargo run -p gelabber-media
```

## Media / SFU (issue 11)

`media/` ist die Binary, nicht mehr nur ein Stub. **webrtc 0.20.5** ist gelockt (0.21 ist RC; str0m wurde nicht gewählt). Room = Sprachkanal. Join nur mit internem 12-Zeichen-Ticket aus Redis (`GETDEL`). RTP wird von Publishern an die anderen Peers im Room weitergereicht. Ein Prozess, kein Mesh, kein Recording, kein LiveKit.

coturn **4.18.0** (`coturn/coturn:4.18.0`) hängt in Compose an 3478/udp+tcp und 49160–49200/udp. Der SFU published 10000–10031/udp (ICE-Lite Host, `MEDIA_ADVERTISED_IP`). Caddy bleibt TCP-only.

Media-WS (nicht der Chat-WS):

| Frame | Bedeutung |
|---|---|
| `{"op":"j","tk":"…"}` | Join mit Ticket |
| `{"op":"o\|a","sdp":"…"}` | SDP offer / answer |
| `{"op":"i","ice":"…","mid?"}` | Trickle-ICE |
| `{"op":"ok","c":"…","u":"…"}` | Ticket akzeptiert |
| `{"op":"err","e":"unauthorized\|bad_request\|…"}` | Abgelehnt |

Persistenz ist auf **sqlx 0.9.0** festgelegt (kein zweites ORM, kein Query-Builder-Mix in v1).

## Web

`npm run dev` in `web/` proxyt `/api` und `/ws` nach `127.0.0.1:8080` und `/media` nach `127.0.0.1:8081` (Vite-Proxy), damit das httpOnly-Cookie same-origin bleibt — genau wie hinter Caddy im Compose. `VITE_API_BASE_URL` ist deshalb relativ (`/api`).

Login-Flow: `GET /api/auth/session` einmal beim Start (parallel zum ersten Render), danach hält ein Zustand-Store den User und der API-Client das CSRF-Token im Speicher. Mit Session öffnet der Tab ein natives WebSocket auf `/ws` (kein Socket.IO); Server- und Kanal-Subscribe folgen der URL, Reconnect nimmt die letzte Seq mit. Voice-Join setzt den lokalen State sofort; Presence bleibt auf `op:"sig"`, SDP/ICE und RTP laufen über `/media/ws` (kurzes Ticket, webrtc 0.20.5). Login/Register schreiben den Store **vor** der clientseitigen Navigation, Logout und Profil-Änderungen sind optimistisch — kein Full-Reload. Feld- und Formfehler erscheinen inline (Client-Regeln spiegeln `api/src/auth/validate.rs`, Server-Feld-Codes werden auf dieselben Texte gemappt). Requests haben 10 s Timeout, Buttons wechseln nur das Label — kein hängender Spinner. Routen: `/`, `/s/…` und `/profile` verlangen einen User, `/login` und `/register` schicken angemeldete User weiter (`?redirect=` für Deep-Links). Stirbt die Session außerhalb des Tabs (Logout woanders, TTL), kippt ein `401 unauthenticated` oder ein Bootstrap mit `user: null` den Store sofort auf anonym und die Seite springt nach `/login?redirect=…`.

Workspace (issue 4): drei Spalten — Server-Rail, Kanal-Sidebar, Seite. Beide Listen sind mit TanStack Virtual virtualisiert (die Sidebar als eine flache Liste aus Kategorie- und Kanalzeilen), damit auch hunderte Einträge ohne Ruckler scrollen. Die Auswahl **ist** die URL (`/s/$serverId/c/$channelId` bzw. `/d/$channelId` für DMs): Klick → Highlight sofort, Details kommen aus dem Query-Cache (`staleTime` 60 s, Prefetch beim Hover über eine Kachel). Die Home-Kachel öffnet Direktnachrichten. Umbenennen, Verschieben, Löschen, Rechte-Toggles und Verlassen/Löschen schreiben zuerst in den Cache und rollen bei einem Fehler mit Toast zurück; Anlegen zeigt eine `tmp:`-Zeile, bis der Server die echte ID liefert. Der zuletzt offene Kanal je Server bleibt lokal gemerkt (`localStorage`). `/s/$serverId/settings`: Name, Mitglieder-Rechte (Checkbox = sofort gespeichert), Einladungen, Mitglieder, Kick/Ban, Sperrliste, Löschen bzw. Verlassen — die Verwaltungs-Sektionen nur mit `manage_server`. `/invite/$code` zeigt Vorschau und „Beitreten“; nicht angemeldete Besucher gehen über Login/Register zurück zum Link. Redirects laufen über `components/Redirect.tsx` (einmal pro Ziel), nicht über `<Navigate>`, das bei jedem Re-Render mit neuem Props-Objekt erneut navigiert.

Chat (issue 5 + 15): Textkanal-Seite virtualisiert die History (TanStack Virtual, dynamische Zeilenhöhe). Senden schreibt die Zeile zuerst in ein kanal-lokales Pending-Overlay (`tmp:`-ID) — sie steht im selben Frame, unter 50 ms, ohne auf GET/POST zu warten. Der Server ersetzt die ID; ein Fehler nimmt die Zeile wieder weg (Toast, keine Geisterzeile). Edit/Delete (eigene oder Mod) sind ebenfalls optimistisch; fremde Deletes kommen per WS und die Liste springt nicht. Scrollen nach oben lädt die nächste ältere Seite per ID-Cursor; am unteren Rand bleibt die Liste kleben, wenn man schon unten war. Anhänge (issue 7): Büroklammer wählt eine Datei, Bild-Preview erscheint sofort; Presign+PUT laufen nach dem Senden im Hintergrund und blockieren den Composer nicht. Voice (issue 12): Join/Leave, Mute/Deafen session-lokal (sofort, dann Sync), Voice-State in der Mitgliederliste mit reservierten Icon-Slots. `join_voice` steuert den Beitritt. Kein LiveKit, kein fremdes SDK.
