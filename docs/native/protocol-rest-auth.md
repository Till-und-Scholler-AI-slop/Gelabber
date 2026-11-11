# Spec: REST API and auth (protocol-rest-auth)

Scope: every `/api/*` route the React web client (`web/src`) calls, cross-checked
against the Axum handlers in `backend/api/src`, plus what a native Rust client
must send to use the existing cookie auth today. No UI layout in this file;
German strings appear only where the REST/auth flow produces them.

Conventions in this file

- Paths are relative to the API origin. The web client prefixes every call with
  `API_BASE = "/api"` (`web/src/api/client.ts`). In the web code `api("/servers")` means
  `GET /api/servers`. This spec always writes the full `/api/...` path.
- `uuid` = lower-case hyphenated UUID string (serde `uuid` default).
- `ts` = RFC 3339 UTC timestamp as chrono serialises `DateTime<Utc>`:
  `2026-11-05T10:20:30.123456Z` (Z suffix, fractional seconds as stored,
  Postgres keeps microseconds). Exception: the `before`/`after` strings of the
  message-context route use `to_rfc3339()` = `+00:00` offset
  (`backend/api/src/messages/workflows.rs:536-539`).
- "nullable" = key always present, value may be `null` (Rust `Option<T>` without
  `skip_serializing_if`). "optional" = key may be absent (`skip_serializing_if`).
- `Body<T>` (`backend/api/src/json.rs:1-50`) = request needs
  `Content-Type: application/json`; missing type, bad syntax, wrong shape or
  read error all map to `400 bad_request` (never 422). Request body fields with
  `#[serde(default)]` may be omitted.
- Path UUIDs that fail to parse answer `404 not_found`, not 400
  (`backend/api/src/path.rs:1-36`).
- The server is the authority for all validation; the web mirrors the rules
  (`web/src/auth/rules.ts`, `web/src/servers/rules.ts`, `web/src/messages/rules.ts`)
  to show errors before a round-trip. The native client should mirror them too
  (constants listed per endpoint).

---

## 1. Transport, headers, cookies, CSRF

### 1.1 Router layout (`backend/api/src/lib.rs:47-77`)

| Layer (outer → inner) | Applies to | Effect |
|---|---|---|
| `hsts` | everything | With `API_COOKIE_SECURE=true` adds `Strict-Transport-Security: max-age=31536000` (`lib.rs:80-101`) |
| `metrics::track`, `TraceLayer` | everything | logging only |
| `limits::gate` | `/api/*`, non-safe methods only | rate limits (§1.6), runs **before** CSRF |
| `csrf::require` | `/api/*`, non-safe methods only | CSRF check (§1.4) |
| handlers | `/api/*` | — |
| outside `/api` | `/health`, `/ready`, `/metrics`, `/ws` | no CSRF, no rate gate. `/ws` = GET upgrade with session cookie (`gateway/mod.rs:43-73`) |

Safe methods = `GET`, `HEAD`, `OPTIONS` (`csrf.rs:35-37`, `limits.rs:433-435`).

### 1.2 What the web client sends on every request (`web/src/api/client.ts:605-644`)

| Item | Value |
|---|---|
| URL | `${API_BASE}${path}` (same origin as the page) |
| `Accept` | `application/json` (always) |
| `Content-Type` | `application/json` when a JSON body is present; for a `Blob` body (avatar, banner, server icon) the blob's type, fallback `application/octet-stream` |
| `X-CSRF-Token` | in-memory token, on every method except `GET` (only when a token is known) |
| credentials | `same-origin` → browser attaches `gelabber_session` and `gelabber_csrf` cookies |
| body | `JSON.stringify(body)` or raw Blob; no body when `body === undefined` |
| timeout | `REQUEST_TIMEOUT_MS = 10_000` per request (`client.ts:69`), via `AbortSignal.timeout`; uploads to the presigned URL use 120 000 ms (`web/src/messages/api.ts:64`) |
| Browser-added | `Origin` (on non-GET), `Sec-Fetch-Site: same-origin`, `Accept-Language` (used by GIF locale, §12) |

Response handling (`client.ts:591-604, 646-671`):

- Every response's `Date` header updates a clock offset; `serverNow() = Date.now() - (localNow - Date)`; second precision. Used for mute countdowns etc.
- Body read as text; empty or non-JSON → `null`.
- Any successful JSON body that contains a string `csrf_token` replaces the in-memory token (`rememberCsrf`, `client.ts:646-655`).
- Non-2xx → `ApiError(code, status, message, fields, retryAfter)` where `code` is the envelope's `error` if it is one of the known codes (§1.5), else `"internal"`. Fetch failure → code `"network"` (status 0); timeout → code `"timeout"` (status 0); caller abort → re-thrown `AbortError`.
- Default TanStack Query policy (`web/src/queryClient.ts:10-14`): mutations never retried; queries use the library default (3 retries, exponential backoff 1 s, 2 s, 4 s; refetch on window focus), with per-query overrides: server detail / DM / profile do not retry `not_found`/`forbidden` and retry at most 2 times (`web/src/servers/queries.ts:95-101`). Stale times: servers & DMs 60 000 ms (`servers/queries.ts:57`, `dms/queries.ts:36`), messages 30 000 ms (`messages/queries.ts:81`), profile cards 15 000 ms (`profile/queries.ts:19`), notification settings and notification privacy 60 000 ms (`messages/notificationSettings.ts:65`, `messages/notificationPrivacy.ts:42`; privacy does not retry a `404` = old server without the route), read state (`GET /api/messages/unread`) 15 000 ms with `retry: false` and no refetch on window focus (`messages/readState.ts:96-99`), `/api/auth/mail` and `/api/gifs` `Infinity` (`auth/mail.ts:21-22` with `retry: false`, `gifs/queries.ts:15`). Search and context queries use `staleTime: 0`.

### 1.3 Cookies (`backend/api/src/cookies.rs`, `auth/mod.rs:49-51, 325-379, 577-579`)

| Cookie | Value | Attributes | Set by | Cleared by |
|---|---|---|---|---|
| `gelabber_session` | 64 lower-case hex chars (32 random bytes, `token.rs:9-13`). Server stores only SHA-256. | `Path=/; Max-Age=<session_ttl secs>; HttpOnly; SameSite=Lax` + `; Secure` when `API_COOKIE_SECURE=true` | login, register (no mail), change-password, verify-email, password-reset confirm | logout, logout-all, register-with-mail (`Max-Age=0`, empty value) |
| `gelabber_csrf` | 64 hex chars | `Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax` (+`Secure`) | `GET /api/auth/session` when the browser has none or an invalid one; **rotated** by every sign-in/sign-out response above | never cleared, only replaced |

Exact header text: `gelabber_session=<tok>; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax[; Secure]` (`cookies.rs:71-80`); a clear is `gelabber_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax[; Secure]` (`cookies.rs:62-64`, test `cookies.rs:149-155`). The CSRF cookie `Max-Age` is `365*24*3600 = 31536000` (`auth/mod.rs:51`). The "valid" check for an existing CSRF cookie is `len == 64 && all ASCII hex digits` (upper-case accepted; `token.rs:16-20`). Cookie parsing on the server: first occurrence of the name in any `Cookie` header wins; exact name match (`cookies.rs:13-24`).

Session lifetimes (`config.rs:80-81, 113-118`; `auth/session.rs:72-141`):

| Setting | Env | Default |
|---|---|---|
| Absolute TTL (cookie `Max-Age`, row upper bound) | `API_SESSION_TTL_HOURS` | 720 h (30 days) |
| Idle window (sliding) | `API_SESSION_IDLE_HOURS` | `min(336 h, TTL)` (14 days); an explicit value > TTL is a startup error (`config.rs:426-438`) |
| Renewal step | — | `min(idle/12, 1 h)`: the row's `expires_at` is pushed to `min(now+idle, created_at+ttl)` on any authenticated request once it would move by ≥ that step |

Any authenticated request (REST or `/ws` upgrade) renews the session; no explicit refresh call exists. An expired/revoked session answers `401 unauthenticated` on routes that need a user; `GET /api/auth/session` answers `user: null`.

### 1.4 CSRF rule (`backend/api/src/csrf.rs:26-59`)

For every `POST/PUT/PATCH/DELETE` under `/api`:

1. If header `Sec-Fetch-Site` equals `cross-site` (case-insensitive) → `403 csrf_invalid`. Absent header passes.
2. Cookie `gelabber_csrf` must be present, else `403 csrf_invalid`.
3. Header `X-CSRF-Token` must be present and valid ASCII, else `403 csrf_invalid`.
4. Cookie value non-empty and constant-time equal to the header → pass. Otherwise `403 csrf_invalid`.

Not checked: `Origin`, `Referer`, `Host`, the token's format (any non-empty equal pair passes the check itself; only `/api/auth/session` re-mints a cookie that is not 64 hex).

This applies to **login and register too**: an anonymous client must first call `GET /api/auth/session` to receive the CSRF cookie + token.

Web retry on CSRF failure (`client.ts:498-563`): when a non-GET answers `403` with `error: "csrf_invalid"`, the client does `GET /api/auth/session`, and if that answer is a valid session payload (`user` null or object with non-empty `id`,`email`,`name`; `csrf_token` non-empty string) it adopts the new token and resends the original request **once**. A second failure is surfaced. For non-auth mutations the retry is aborted (`AbortError`, "The session changed. Try again.") if the bootstrap reveals a different user id than the one that started the mutation.

### 1.5 Error envelope (`backend/api/src/error.rs`)

```json
{"error":"validation_failed","message":"Some fields are invalid.","fields":{"email":"invalid"},"retry_after":12}
```

| Field | Type | Presence |
|---|---|---|
| `error` | string code (table below) | always |
| `message` | English fallback string | always |
| `fields` | object `{field: field_code}` | only when non-empty (`skip_serializing_if = BTreeMap::is_empty`) |
| `retry_after` | integer seconds | only for `rate_limited` and `upstream_quota`, where the same value is also sent as `Retry-After` header; and (v0.10) for `join_request_declined`, in the body only |

| `error` | HTTP | `message` (server, English) | German UI text (`web/src/auth/rules.ts:369-413`) |
|---|---|---|---|
| `validation_failed` | 422 | Some fields are invalid. | Bitte die markierten Felder prüfen. |
| `bad_request` | 400 | Request body could not be read. | Da ist etwas schiefgelaufen. Bitte noch einmal versuchen. |
| `unauthenticated` | 401 | Sign in required. | Bitte melde dich erneut an. |
| `invalid_credentials` | 401 | E-mail or password is wrong. | E-Mail oder Passwort ist falsch. |
| `csrf_invalid` | 403 | Missing or invalid CSRF token. | Sicherheits-Token abgelaufen. Bitte noch einmal versuchen. |
| `insecure_transport` | 403 | Sign-in needs HTTPS. Open the app over https:// (or http://localhost on this machine). | Anmelden geht nur über eine verschlüsselte Verbindung. Öffne Gelabber mit https:// (oder auf diesem Rechner über http://localhost). |
| `forbidden` | 403 | per call site, e.g. `You need the manage_server permission.` (`servers/membership.rs:64-75`) | Dafür fehlt dir die Berechtigung. |
| `not_found` | 404 | Not found. | Das gibt es nicht (mehr). |
| `email_taken` | 409 | This e-mail address is already registered. (also `fields.email = "taken"`) | Diese E-Mail-Adresse ist schon registriert. |
| `email_unverified` | 403 | Confirm your e-mail address first. We sent you a new link. | Bitte bestätige zuerst deine E-Mail-Adresse. Wir haben dir gerade einen neuen Link geschickt. |
| `token_invalid` | 410 | This link has expired or has already been used. | Dieser Link ist abgelaufen oder wurde schon benutzt. |
| `theme_conflict` | 409 | Themes changed on another device. Reload and try again. | Auf einem anderen Gerät geändert. Bitte erneut versuchen. |
| `invite_invalid` | 410 | This invite link has expired or been used up. | Diese Einladung ist abgelaufen oder aufgebraucht. |
| `banned` | 403 | You are banned from this server. | Du bist auf diesem Server gesperrt. |
| `rate_limited` | 429 | Too many requests. Try again in a moment. | Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen. |
| `quota_exceeded` | 429 | Daily upload quota reached. | Tageslimit für Uploads erreicht. |
| `upstream_unavailable` | 502 | The GIF service did not answer. Try again in a moment. | Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen. |
| `upstream_quota` | 503 | This server has used up its GIF requests for now. Try again later. | Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen. |
| `internal` | 500 | Something went wrong on our side. | Da ist etwas schiefgelaufen. Bitte noch einmal versuchen. |
| `account_disabled` | 403 | This account has been disabled. | v0.10: catalog `auth` `error.accountDisabled` |
| `invitation_required` | 403 | Registration needs an invitation. (also `fields.invitation = "required"`) | v0.10: `auth` `error.invitationRequired` |
| `invitation_invalid` | 410 | This invitation is unknown, expired or already used. (also `fields.invitation = "invalid"`) | v0.10: `auth` `error.invitationInvalid` |
| `server_blocked` | 403 | This server has been blocked by the operator. | v0.10: `auth` `error.serverBlocked` |
| `join_request_required` | 409 | This server takes requests to join. | v0.10: `auth` `error.joinRequestRequired` |
| `join_request_declined` | 409 | Your request to join was not accepted. You can ask again later. (also `retry_after`: seconds until the cooldown ends) | v0.10: `auth` `error.joinRequestDeclined`, or `error.joinRequestDeclinedUntil` with the date |
| `join_requests_full` | 409 | Too many open requests to join. Try again later. | v0.10: `auth` `error.joinRequestsFull` |
| `already_member` | 409 | You are already a member of this server. | v0.10: `auth` `error.alreadyMember` |
| client-only `network` | 0 | Network error. | Keine Verbindung zum Server. |
| client-only `timeout` | 0 | Request timed out. | Der Server antwortet nicht. Bitte noch einmal versuchen. |

Non-JSON error bodies exist: `DefaultBodyLimit` overflow on raw-byte routes (avatar/banner/icon) and axum's own 405/404-for-unknown-route answer plain text; the web maps any unknown/unparseable body to code `internal` with the real HTTP status (`client.ts:349-353, 453-462`, test `client.test.ts:119-124`).

Forms (`web/src/auth/useFormErrors.ts:19-23, 65-95`): `rate_limited` with a non-zero `retry_after` → form text `Zu viele Versuche. Bitte ${ceil(seconds)} Sekunden warten.`, no field errors, and a cooldown of that many seconds that disables submit; otherwise field codes → per-field text. Form line by number of field errors: **0** → `errorMessage(code)` (table above); **exactly 1** → no form line at all (focus moves to the invalid field); **> 1** → `Bitte die markierten Felder prüfen.`. A thrown non-`ApiError` → form line `Da ist etwas schiefgelaufen. Bitte noch einmal versuchen.`; an `AbortError` → errors left unchanged.

Field codes (`web/src/api/client.ts:8-20`): `required`, `invalid`, `too_short`, `too_long`, `taken`, `wrong`, `stale`, `exclusive`, `unavailable`, `not_editable`, `too_large`, `mismatch`; the server also emits `limit` (reactions) and the fields `attachment_ids.N` (N = 0..9). German per-field copy (`web/src/auth/rules.ts:281-366`):

| Code | Text |
|---|---|
| `required` | `${label} fehlt.` |
| `too_short` | password: `Mindestens 8 Zeichen.`; else `${label} ist zu kurz.` |
| `too_long` | size: `Die Datei ist zu groß (max. 25 MB).`; attachment_ids: `Höchstens 10 Dateien pro Nachricht.`; else `${label} ist zu lang.` |
| `too_large` | `Alle Dateien zusammen sind zu groß (max. 100 MB).` |
| `mismatch` | `Der Inhalt der Datei passt nicht zu ihrem Typ.` |
| `taken` | `Diese E-Mail-Adresse ist schon registriert.` |
| `wrong` | `Das Passwort stimmt nicht.` |
| `stale` | `Dein Bild wurde inzwischen woanders geändert. Lade die Seite neu.` |
| `exclusive` | `Ein GIF wird ohne Text und Anhang gesendet.` |
| `unavailable` | `GIFs sind auf diesem Server nicht eingerichtet.` |
| `not_editable` | `Ein GIF lässt sich nicht bearbeiten.` |
| `invalid` | email: `Das sieht nicht wie eine E-Mail-Adresse aus.`; avatar_url: `Bitte eine vollständige https-URL angeben.`; accent_color/banner_color: `Bitte eine Farbe im Format #rrggbb wählen.`; bio: `Nur Text und Zeilenumbrüche, keine Steuerzeichen.`; pronouns/status_text: `Nur eine Zeile Text, keine Steuerzeichen.`; content_type: `Dieser Dateityp ist nicht erlaubt.`; else `${label} ist ungültig.` |

Labels: email `E-Mail-Adresse`, password `Passwort`, current_password `Aktuelles Passwort`, name `Name`, avatar_url `Avatar-URL`, bio `Über mich`, pronouns `Pronomen`, status_text `Status`, accent_color `Akzentfarbe`, banner_color `Bannerfarbe`, banner_url `Banner`, avatar_decoration `Avatar-Rahmen`, kind `Kanaltyp`, category_id `Kategorie`, member_permissions `Rechte`, max_uses `Maximale Nutzungen`, expires_in_hours `Gültigkeit`, content `Nachricht`, before/after `Cursor`, limit `Anzahl`, user_id `Nutzer`, filename `Dateiname`, content_type `Dateityp`, size `Datei`, attachment_ids `Anhang`, gif `GIF`, q `Suche`, token `Link`; unknown field → `Eingabe`.

Not-found semantics: a server/channel/message the caller cannot see is `404`, never `403` (`error.rs:14-16`). `403 forbidden` only appears once membership is proven.

### 1.6 Rate limits (`backend/api/src/limits.rs`)

All on non-safe methods unless noted; windows are in-process per API node. Values are the defaults (`limits.rs:34-56`); each is overridable by env (`API_RATE_AUTH_PER_MIN`, `API_RATE_API_PER_MIN`, `API_RATE_MSG_PER_MIN`, `API_RATE_UPLOAD_PER_HOUR`, `API_RATE_GIF_PER_MIN`, `API_RATE_GIF_MEDIA_PER_MIN`, `API_RATE_PREVIEW_PER_MIN`; `config.rs:40-65, 770-800`), and `0` disables a bucket. The gate matches by path shape (`limits.rs:330-335`): `up` = any `POST …/attachments` or `PUT /api/me/avatar|banner|/api/servers/*/icon`; `msg` = any `POST …/messages`. Logout, logout-all and `/auth/password` are **not** in the `auth` bucket; they count against `api`.

| Bucket | Key | Limit | Routes |
|---|---|---|---|
| `auth` | client IP (IPv6 /64; last `X-Forwarded-For` entry, else peer) | 20 / min | POST `/api/auth/login`, `/register`, `/verify-email`, `/password-reset`, `/password-reset/confirm` (`limits.rs:422-431`) |
| `login` (failures) | account e-mail | 10 failures / 15 min → 429 | login, change-password, verify-email wrong password |
| `api` | `u:{user_id}` if cookie is a live session, else IP | 180 / min | every other mutation |
| `msg` | same | 60 / min (plus `api`) | `POST …/messages` |
| `up` | same | 60 / hour (plus `api`) | `POST …/attachments`, `PUT /api/me/avatar`, `PUT /api/me/banner`, `PUT /api/servers/{id}/icon` |
| upload bytes | user, Postgres | 1 GiB / day → `429 quota_exceeded` | presign, avatar, banner, icon |
| `read_bulk` | user | 20 / min | `PUT /api/servers/{id}/read`, `PUT /api/messages/read-all` (`messages/workflows.rs:363-378`) |
| `gif` | user | 30 / min, **also on GET** | `/api/gifs/trending`, `/api/gifs/search`, GIF sends |
| `gif_media` | user | 240 / min, GET | `/api/gifs/media` |
| account mail | address | 5 / hour (silently not sent) | register, login of unverified, password-reset |
| `report` | user | 10 / hour → `429 rate_limited` | `POST /api/servers/{id}/reports` (v0.10) |
| `mail_invite` | invited address | 5 / hour (silently not sent; the invitation is still created) | `POST /api/operator/invitations` with `send_mail` (v0.10) |
| `dir_join` | user | 10 / hour → `429 rate_limited`, shared by both routes | `POST /api/directory/listings/{id}/join` and `/request` (v0.10, §4.7) |

A native client must not send `X-Forwarded-For` (the API trusts the last entry).

---

## 2. Session bootstrap, login, register, logout (web flow)

Source: `web/src/auth/session.ts`, `web/src/api/client.ts`, backend `auth/mod.rs`.

### 2.1 State machine the native client should reproduce

| State | Meaning |
|---|---|
| `unknown` | not yet asked, or a check failed |
| `authenticated` | `user` object known |
| `anonymous` | server said `user: null`, or any request got `401 unauthenticated` |

1. **App start** (`session.ts:189-233`): `GET /api/auth/session`. Store `csrf_token`. If `user` non-null → `authenticated` (and load workspace); else `anonymous` → login screen.
2. **Check failed** (network, timeout, 5xx): stay `unknown`, show `Deine Sitzung konnte nicht geprüft werden. Prüfe deine Verbindung und versuche es erneut.` (`session.ts:221`) and retry automatically after `SESSION_RETRY_MS = [2000, 5000, 15000, 30000]` ms (index = `min(max(failures,1),4) - 1`, i.e. capped at 30 s; `session.ts:44-49`) and immediately on "online"/window visible (`session.ts:44-105`). Recovery notice copy (`components/SessionRecoveryNotice.tsx:70-106`): h1 `Server antwortet nicht` (online) / `Keine Verbindung zu Gelabber` (offline); lead `Gelabber ist nicht erreichbar. Möglicherweise startet der Server neu.` / `Dein Gerät ist offline. Sobald es wieder verbunden ist, geht es weiter.`; visible status `Wird geprüft…` while checking, else `Neuer Versuch in ${seconds} s` (live countdown) or nothing at 0; screen-reader status `Neuer Versuch in ${planned} Sekunden`; primary large button `Jetzt erneut versuchen` (pending label `Wird geprüft…`) that retries at once.
3. **Login** (`session.ts:248-269`): `POST /api/auth/login {email, password}` → `SessionResponse`; adopt `user` and the rotated `csrf_token`; drop every cache of a different previous user.
4. **Register** (`session.ts:275-281`): `POST /api/auth/register {email, password, name, locale}`; `locale` is the language the device shows (`de` \| `en`, from v0.10). `201` = signed in (like login). `202` with `verification_sent: true` and `user: null` = account mail is on, nobody signed in; show "Postfach prüfen" screen (`pages/RegisterPage.tsx:165-222`). From v0.10 the body may also carry `invitation` and `server_invite` (invitation-only instances, see §2.2 and `GET /api/auth/registration`).
5. **Logout** (`session.ts:317-352`): optimistic — switch to `anonymous` first, then `POST /api/auth/logout` (no body). On failure show `Die Abmeldung auf dem Server konnte nicht bestätigt werden. Bitte versuche es erneut.` and remember the user id to retry. `logout-all` identical with `POST /api/auth/logout-all`.
6. **Any 401 `unauthenticated`** on a normal request → switch to `anonymous` (`client.ts:478-484`). Login page then may show `Sitzung abgelaufen. Melde dich erneut an.` (`pages/LoginPage.tsx:70-74`, reason `expired`).
7. **Change password** (`session.ts:358-367`): `POST /api/auth/password {current_password, password}` → `SessionResponse` with a fresh session cookie for this client; all other sessions of the account (and their `/ws` sockets and media tickets) are revoked. Caches stay (same account).
8. **Profile edits** are optimistic with rollback (`session.ts:406-442`); one profile write in flight at a time (`claimProfileWrite`, `session.ts:384-404`).

Browser-only machinery that a native client does **not** need: the `localStorage` key `gelabber:auth-generation` fence and `navigator.locks` lock `gelabber:auth-cookies` (`client.ts:103-220`) coordinate tabs sharing one cookie jar. A native client should still serialise auth-changing requests (`/auth/session`, login, register, verify-email, password-reset/confirm, `/auth/password`, logout, logout-all) and discard responses of a request started under a previous identity.

### 2.2 Auth endpoints

`SessionResponse` (`auth/mod.rs:178-182`): `{ "user": User | null, "csrf_token": string, "operator": bool }`. `operator` (v0.10) is `true` when the signed-in account is an instance operator (`API_OPERATOR_EMAILS`; with account mail on, only once its address is confirmed); the clients then offer "Betrieb" (§4.6). Servers older than v0.10 omit it: read a missing key as `false`. The `202` register answer has no `operator`.
`User` (`auth/user.rs:11-38`, flattened card): see §3.1.

| Method & path | Web caller | Request | Success | Errors |
|---|---|---|---|---|
| `GET /api/auth/session` | `session.ts:210`, CSRF retry `client.ts:528` | — (cookies) | `200 SessionResponse`. Sets `gelabber_csrf` only when the request had no valid (64-hex) one (`auth/mod.rs:555-575`). Never 401. | — |
| `GET /api/auth/registration` | v0.10 register page | — (no session needed) | `200 {"mode": "open" \| "invitation"}` (`API_REGISTRATION`). `invitation`: a new account needs a beta invitation or the invite code of a server that is not blocked | — (`404` from servers older than v0.10: treat as `open`) |
| `GET /api/auth/mail` | `auth/mail.ts:20` (login page, `staleTime: Infinity`, no retry) | — | `200 {"enabled": bool}` — login page (`pages/LoginPage.tsx:136-148`): `true` → link `Passwort vergessen?` to the forgot-password screen (prefilled with the typed e-mail); `false` → plain text `Passwort vergessen? Frag die Person, die diesen Gelabber-Server betreibt.`; unknown/failed → nothing | — |
| `POST /api/auth/register` | `session.ts:280` | `{email, password, name}` all strings, default `""` (`auth/mod.rs:79-87`); optional `locale` (`"de"` \| `"en"`); optional `invitation` (beta invitation code) and `server_invite` (a server invite code, e.g. from an `/invite/{code}` redirect), both ignored while registration is `open`. In `invitation` mode a valid beta invitation is used up by the new account (with mail on: only by a new or taken-over pending registration, never by a confirmed address); a server invite admits without being used up, and joining still takes `POST /api/invites/{code}/join`. Without `locale` the `Accept-Language` header decides (first range by `q` whose primary subtag is `de` or `en`; only other languages → `en`), without a usable header `de` (clients older than v0.10). Taking over a pending registration (mail on) overwrites its `locale` like its name and password; a confirmed account keeps its own | no mail: `201 SessionResponse` + both cookies. Mail on: `202 {"user":null,"csrf_token":…,"verification_sent":true}`, session cookie cleared, CSRF rotated — identical answer whether the address existed or not | `422` fields `email` required/too_long/invalid, `name` required/too_long/invalid, `password` required/too_short/too_long, `locale` invalid (present but not `de`/`en`); `403 invitation_required` (invitation mode, no code); `410 invitation_invalid` (unknown, used, revoked, expired or for another address; a server invite that is expired, used up or of a blocked server) — both after the `422` checks and before any account lookup; `409 email_taken` (no mail only); `403 insecure_transport`; `429` |
| `POST /api/auth/login` | `session.ts:268` | `{email, password}` | `200 SessionResponse` + cookies | `422` email codes / `password: required`; `401 invalid_credentials` (also for unknown e-mail and for password > 128 chars); `403 account_disabled` (v0.10: an operator disabled the account; only after the correct password, before `email_unverified`, no mail is sent); `403 email_unverified` (mail on, after correct password; a new link is mailed); `429` (`login_blocked` or `auth` bucket); `403 insecure_transport` |
| `POST /api/auth/logout` | `session.ts:335` | none | `200 {"csrf_token": string}`; clears session cookie, rotates CSRF. Works without a session. | `403 csrf_invalid` |
| `POST /api/auth/logout-all` | `session.ts:323` | none | `200 {"csrf_token"}`; every session of the account deleted, sockets closed | `401` without session |
| `POST /api/auth/password` | `session.ts:362` | `{current_password, password}` | `200 SessionResponse` + new cookies | `422 current_password: required\|wrong`, `password` register rules; `401`; `429`; `403 insecure_transport` |
| `POST /api/auth/verify-email` | `session.ts:291` (route `/verify-email#token=<64hex>`, token parsed by `/(?:^#\|&)token=([0-9a-f]{64})(?:&\|$)/i` and lower-cased, `auth/mail.ts:27-33`) | `{token, password}` | `200 SessionResponse` + cookies | checked in this order (`auth/mod.rs:447-480`): `403 insecure_transport`; `410 token_invalid` (unknown/used/expired token — checked **before** the password); `422 password: required`; `429` (login lockout of the account); `422 password: wrong` (> 128 chars also `wrong`); `410` again if the token was replaced meanwhile; `403 account_disabled` (v0.10: the address is confirmed and the token used, but nobody is signed in) |
| `POST /api/auth/password-reset` | `session.ts:310` | `{email}` | `202 {}` always (unknown address too); a link is mailed only for an existing account and within 5 mails/h | `404 not_found` when instance has no mail (checked first); `422 email …`; `429` auth bucket. **No** transport check. |
| `POST /api/auth/password-reset/confirm` | `session.ts:302` (route `/reset-password#token=<64hex>`, `mail.rs:193`) | `{token, password}` | `200 SessionResponse` + cookies; all other sessions revoked, address counts as confirmed | `403 insecure_transport`; `422 token: required` and/or `password required\|too_short\|too_long`; `410 token_invalid`; `403 account_disabled` (v0.10: after the token check, before anything is written; the password stays unchanged); `429` auth bucket |

Validation constants (`auth/validate.rs`; web mirror `web/src/auth/rules.ts:8-16`):

| Field | Rule |
|---|---|
| email | trimmed + lower-cased; ≤ 254 chars; exactly one `@`, non-empty local part, domain contains `.`, does not start/end with `.`, no whitespace/control chars |
| password | 8..=128 Unicode scalar values (not trimmed) |
| name (user) | trimmed, 1..=64 chars, no control chars |

`transport_allowed` (`auth/mod.rs:543-552`): routes register, login, password, verify-email, password-reset/confirm answer `403 insecure_transport` when **both** `API_COOKIE_SECURE` and `API_ALLOW_PLAIN_HTTP` are false **and** the request has a `Host` header that is not loopback (`localhost`, `*.localhost`, `127.0.0.0/8`, `[::1]`, any port, case-insensitive; `cookies.rs:26-41`). No `Host` header → allowed. The check runs **before** body validation (an empty login on a LAN IP answers 403, not 422). `logout`, `logout-all`, `session`, `mail` and `password-reset` (request) have no transport check.

Check order in `login` (`auth/mod.rs:273-320`): transport → `422` (email rules, `password: required`) → `429` login lockout for the address → password > 128 chars = `401 invalid_credentials` (counts as a failure) → Argon2 verify (unknown address runs against a dummy hash; failure counts) → `403 account_disabled` (v0.10) → `403 email_unverified` (mail on only; mails a new link if within 5/h) → `200`. In `register`: transport → `422` → `403 invitation_required` / `410 invitation_invalid` (v0.10, invitation mode only) → (no mail) insert → `409 email_taken` or `201`. Every sign-in ends in `signed_in`, which refuses a disabled account with `403 account_disabled`; a disabled account's sessions are deleted, so its old cookie answers like no session (`user: null`, `401`). A previous session cookie the request carried is revoked on every sign-in (`signed_in`, `auth/mod.rs:325-351`).

---

## 3. Me / profile / avatars / themes

### 3.1 Shapes

`User` (own account; `backend/api/src/auth/user.rs:11-38`; web `web/src/auth/types.ts:20-26`)

| Key | Type | Notes |
|---|---|---|
| `id` | uuid | |
| `email` | string | lower-case |
| `name` | string | |
| `avatar_url` | nullable string | `/api/avatars/{user}/{file}` (upload) **or** any `https://…` URL (legacy link). Web renders with `referrerPolicy="no-referrer"` (`components/Avatar.tsx:42-47`) |
| `created_at` | ts | |
| `bio` | nullable string | ≤ 190 chars, may contain `\n` |
| `pronouns` | nullable string | ≤ 40 |
| `status_text` | nullable string | ≤ 80 |
| `accent_color` | nullable string | `#rrggbb` lower-case |
| `banner_color` | nullable string | `#rrggbb` lower-case |
| `banner_url` | nullable string | only `/api/banners/{user}/{file}` |
| `avatar_decoration` | nullable string | `ring` \| `glow` \| `rainbow` \| `stars` |
| `locale` | string | `de` \| `en` (migration 0030, v0.10): the account's language for both clients, push and mail. Missing from servers older than v0.10 |

All card keys are always present (serialised `null` when unset). Key order on the wire: as listed.

`PublicProfile` (`auth/user.rs:42-51`): same as `User` without `email` and `locale`; member lists do not carry `locale` either.

### 3.2 Endpoints

| Method & path | Web caller | Request | Success | Errors / notes |
|---|---|---|---|---|
| `GET /api/me` | not called by web (session payload is used) | — | `200 User` | 401 |
| `PATCH /api/me` | `session.ts:425` | any subset of `{name, avatar_url, bio, pronouns, status_text, accent_color, banner_color, banner_url, avatar_decoration}` (all `Option<String>`, `profile.rs:26-39`). Absent or `null` = unchanged; `""` = clear (not allowed for `name`). | `200 User` (fresh row). Empty/no-op patch returns current user. | `422` codes: name required/too_long/invalid; avatar_url too_long(>2048)/invalid(not `https://host…`)/stale (an `/api/avatars/…` URL other than the current one); bio/pronouns/status_text too_long/invalid (control or bidi chars U+202A–202E, U+2066–2069); colours invalid; banner_url invalid/stale (only `""` or the current value accepted); avatar_decoration invalid. `401` if the row vanished. |
| `GET /api/users/{id}` | `profile/queries.ts:22` (profile card dialog) | — | `200 PublicProfile` | `404` unless viewer is the user, shares a server, or has a DM with them (`auth/user.rs:53-88`) |
| `PUT /api/me/avatar` | `session.ts:450,460` | raw image bytes; web sends 256×256 WebP (quality 0.85), else JPEG 0.85 or PNG (`web/src/auth/avatarImage.ts:2-15,87-114`); `Content-Type` = blob type, server ignores it and sniffs | `200 User` with new `avatar_url` | `400 bad_request` not PNG/JPEG/WebP or declared size > 256×256; body > 512 KiB → 413 (plain text); `429` up bucket / `quota_exceeded`; not optimistic |
| `PUT /api/me/banner` | `session.ts:455` | raw bytes, web 960×320 WebP/JPEG | `200 User` with new `banner_url` | as avatar, max 960×320, body ≤ 1 MiB |
| `GET /api/avatars/{user}/{file}` | `<img src>` | cookie | `200` image bytes; `Content-Type` sniffed (`image/png|jpeg|webp`), `Cache-Control: private, max-age=31536000, immutable`, `X-Content-Type-Options: nosniff` (`avatars.rs:327-363`) | `401` without session; `404` unknown/oversized. Any signed-in user may load any avatar. |
| `GET /api/banners/{user}/{file}` | `<img src>` | cookie | same headers | `404` unless the card is visible to the viewer (same rule as `/api/users/{id}`) |
| `PUT /api/me/locale` | `pages/AppearanceSettings.tsx` (Sprache) | `{"locale":"en"}` | `200 {"locale":"en"}`; every socket of the account gets `{"op":"lc","l":"en"}` (protocol-gateway §3). Push titles and account mail follow from the next one on | `422 validation_failed` with `fields.locale` `required` (missing or `null`) \| `invalid`; `401 unauthenticated` (also if the row vanished); `403 csrf_invalid` |

### 3.3 Account themes (`backend/api/src/themes.rs`; web `web/src/theme/account.ts:119-240`, `web/src/theme/model.ts:23-30`)

Keys are **camelCase** here (only place in the API). `deny_unknown_fields` on every object: an extra key → `400 bad_request`.

`ThemeDocument`:

| Key | Type | Rule |
|---|---|---|
| `version` | number | must be `1` |
| `revision` | integer | ≥ 0, < 2^53−1; the server's revision you are based on |
| `active` | string | one of the built-ins `dark`, `light`, `system`, `catppuccin`, `catppuccin-latte`, `nord`, `gruvbox`, `tokyo-night`, `everforest`, `kanagawa`, `rose-pine`, `flexoki-light`, `osaka-jade`, `matte-black`, `ristretto` (`themes.rs:13-29`) or the id of an entry in `customThemes` |
| `customThemes` | array of Theme, ≤ 50 | unique ids |

`Theme`: `{version: 1, id: "custom-<uuid>" (exactly 43 chars), name (trimmed non-empty, ≤ 60 chars, no control chars), mode: "light"|"dark", style: "clear"|"soft"|"terminal", colors: {background, panel, surface, rail, text, muted, accent, border}}`; every colour `#` + 6 hex digits.

| Method & path | Request | Success | Errors |
|---|---|---|---|
| `GET /api/me/themes` | — | `200 ThemeDocument`; account without row → `{"version":1,"revision":0,"active":"dark","customThemes":[]}` | 401 |
| `PUT /api/me/themes` | full `ThemeDocument` with the `revision` last read; body ≤ 64 KiB (`themes.rs:128`) | `200 ThemeDocument` with `revision` = old + 1 | `400 bad_request` invalid document, unknown key, or body > 64 KiB (the JSON extractor maps the length-limit rejection to the envelope, not a plain 413); `409 theme_conflict` when the stored revision ≠ sent revision (or a row exists and sent revision is 0) |

Web flow: on sign-in `GET`; if `revision === 0` the device's current local choice is imported via a `PUT` (`account.ts:133-140`). Each change is applied locally first, then `PUT`. While a locally cached change is still pending at load, the status line is `Noch nicht synchronisiert.` (`account.ts:93`). On `409` the UI state is `conflict` with `Auf einem anderen Gerät geändert. Deine Änderung bleibt erhalten. Erneut anwenden übernimmt sie auf den aktuellen Stand.`; "retry" does `GET` then re-applies the pending operation and `PUT`s (`account.ts:180-233`). Other failures: `Noch nicht synchronisiert. Deine Änderung ist hier gespeichert.`; load failure: `Themes konnten nicht geladen werden. Bitte erneut versuchen.`; malformed server document: `Unbekanntes Theme-Format vom Server.` / `Unbekanntes Theme-Format.`; deleting the active custom theme falls back to `active: "dark"`.

---

## 4. Servers, categories, channels, members

### 4.1 Shapes

`Permission` names, in the canonical order the server serialises (`servers/permissions.rs:28-37`): `manage_server`, `manage_channels`, `manage_messages`, `mention_everyone`, `send_messages`, `send_files`, `join_voice`, `go_live`. Bits: manage_server 1, manage_channels 2, send_messages 4, send_files 8, join_voice 16, go_live 32, manage_messages 64, mention_everyone 128. New server default member mask = `send_messages, send_files, join_voice, go_live` (60). Owner always holds all eight.

German labels (`web/src/servers/permissions.ts:6-51`):

| id | label | hint |
|---|---|---|
| manage_server | Server verwalten | Umbenennen, Rechte ändern, Einladungen, Kick und Ban. |
| manage_channels | Kanäle verwalten | Kategorien und Kanäle anlegen, umbenennen, löschen. |
| manage_messages | Nachrichten löschen | Fremde Nachrichten im Chat löschen. |
| mention_everyone | @everyone und @here | Alle Mitglieder auf einmal erwähnen und benachrichtigen. |
| send_messages | Nachrichten schreiben | In Textkanälen schreiben. |
| send_files | Dateien senden | Anhänge hochladen. |
| join_voice | Sprachkanälen beitreten | In Sprachkanälen reden, zuhören und die Kamera zeigen. |
| go_live | Live gehen | Im Sprachkanal einen Bildschirm oder ein Fenster für alle dort zeigen. |

`ServerView` (`servers/mod.rs:69-79`; web `Server`, `web/src/servers/types.ts:20-33`):

| Key | Type |
|---|---|
| `id` | uuid |
| `name` | string (1..=100) |
| `owner_id` | uuid |
| `icon_url` | nullable string `/api/server-icons/{server}/{file}` |
| `created_at` | ts |
| `role` | `"owner"` \| `"member"` |
| `permissions` | Permission[] — caller's effective set (all eight for owner) |
| `member_permissions` | Permission[] — mask every non-owner holds |
| `blocked` | bool (v0.10) — an operator blocked the server (§4.5). Missing from servers older than v0.10: read as `false` |

`Category` (`servers/channel.rs:80-86`): `{id, server_id, name, created_at}`.
`Channel` (`servers/channel.rs:88-96`): `{id, server_id: uuid (nullable; null only for DM channels), category_id: nullable uuid, name, kind: "text"|"voice"|"dm", created_at}`.
`Member` (`servers/mod.rs:100-118`, flattened): `{user_id, name, avatar_url: nullable, joined_at: ts, status_text: nullable, accent_color: nullable, avatar_decoration: nullable, role: "owner"|"member"}`.
`ServerDetail` (`servers/mod.rs:121-128`): `ServerView` fields flattened + `categories: Category[]`, `channels: Channel[]`, `members: Member[]`.

Ordering (server-side, the client must not re-sort differently): servers by the caller's `joined_at`, then id (`servers/mod.rs:148-153`); categories and channels by `created_at`, then id (`servers/channel.rs:99-111`); members by `joined_at`, then user_id (`servers/mod.rs:263-275`). There is no position/reorder field. Channels with `category_id: null` are uncategorised.

New server content (`servers/mod.rs:194-213`): category `Textkanäle` with text channel `allgemein`, category `Sprachkanäle` with voice channel `Wohnzimmer`.

### 4.2 Endpoints (`web/src/servers/api.ts`)

| Method & path | Web fn (line) | Request | Success | Permission / errors |
|---|---|---|---|---|
| `GET /api/servers` | `listServers` (18) | — | `200 ServerView[]` | 401 |
| `POST /api/servers` | `createServer` (24) | `{name}` | `201 ServerDetail` | `422 name required/too_long/invalid` |
| `GET /api/servers/{id}` | `getServer` (21) | — | `200 ServerDetail` | `404` non-member |
| `PATCH /api/servers/{id}` | `updateServer` (27) | `{name?, member_permissions?: Permission[]}` (null/absent = unchanged; `[]` = no permissions) | `200 ServerView` | `manage_server`; `422 name …`, `member_permissions: invalid` (unknown name) |
| `DELETE /api/servers/{id}` | `deleteServer` (37) | — | `204` | owner only (`403 Only the server owner can do this.`) |
| `POST /api/servers/{id}/leave` | `leaveServer` (40) | none | `204` | owner → `403 The owner cannot leave; delete the server instead.` |
| `PUT /api/servers/{id}/icon` | `uploadServerIcon` (31) | raw image ≤ 512 KiB, ≤ 256×256, PNG/JPEG/WebP (web: 256×256 WebP via the avatar preparer) | `200 ServerView` | `manage_server`; `400`; `429` up bucket |
| `DELETE /api/servers/{id}/icon` | `removeServerIcon` (34) | none | `200 ServerView` | `manage_server` |
| `GET /api/server-icons/{server}/{file}` | `<img>` (`components/ServerIcon.tsx:18`) | cookie | image, same headers as avatars | `401`; any signed-in user |
| `POST /api/servers/{id}/categories` | `createCategory` (43) | `{name}` | `201 Category` | `manage_channels`; `422 name` (1..=100, no control) |
| `PATCH /api/categories/{id}` | `updateCategory` (49) | `{name?}` | `200 Category` | `manage_channels` |
| `DELETE /api/categories/{id}` | `deleteCategory` (52) | — | `204`; its channels stay with `category_id: null` | `manage_channels` |
| `POST /api/servers/{id}/channels` | `createChannel` (55) | `{name, kind: "text"\|"voice", category_id: uuid-string \| ""}` (web always sends `category_id`, `""` for none; absent/null also = none; `kind` absent = text) | `201 Channel` | `manage_channels`; `422 kind: invalid` (incl. `"dm"`), `name required/too_long/invalid`, `category_id: invalid` (unparseable or other server) |
| `PATCH /api/channels/{id}` | `updateChannel` (65) | `{name?, category_id?}`; `category_id: ""` detaches, absent/null = unchanged | `200 Channel` | `manage_channels` |
| `DELETE /api/channels/{id}` | `deleteChannel` (76) | — | `204`; voice occupants and tickets are revoked | `manage_channels` |

Text-channel name normalisation (server, `servers/validate.rs:30-62`; web preview `slugifyChannelName`, `web/src/servers/rules.ts:20-45`): after trim/length check, lower-case; whitespace, `_` and `-` runs → one `-`; `#` removed; no leading/trailing `-`; empty result → `name: invalid`. Voice names are kept as typed (trimmed).

### 4.3 Members, moderation (`servers/moderation.rs`)

`Ban`: `{user_id, name, avatar_url: nullable, banned_by: uuid, banned_at: ts}`, ordered by `banned_at`, user_id.

| Method & path | Web fn | Request | Success | Errors |
|---|---|---|---|---|
| `POST /api/servers/{id}/kick` | `kickMember` (94) | `{user_id: string}` | `204` | `manage_server`; `422 user_id: required` (empty); `404` unparseable/not a member; `403 You cannot kick or ban yourself.` / `403 The server owner cannot be kicked or banned.` |
| `POST /api/servers/{id}/ban` | `banMember` (100) | `{user_id}` | `204` (idempotent) | as kick; `404` unknown user |
| `GET /api/servers/{id}/bans` | `listBans` (106) | — | `200 Ban[]` | `manage_server` |
| `DELETE /api/servers/{id}/bans/{user_id}` | `unbanMember` (109) | — | `204` | `manage_server`; `404` not banned |

Member list itself comes only inside `ServerDetail.members`; live changes arrive over `/ws`.

### 4.4 Invites (`servers/invite.rs`)

Code: 12 chars from `abcdefghjkmnpqrstuvwxyz23456789` (`backend/shared/src/ticket.rs:8-9`); input is trimmed and lower-cased before lookup; anything else → `404`. Shareable link the web shows: `${window.location.origin}/invite/${code}` (`servers/api.ts:113-115`); a native client should build `<instance origin>/invite/<code>` and accept such links (path segment after `/invite/`).

`Invite`: `{code, server_id, created_by: uuid, created_at, expires_at: nullable ts, max_uses: nullable int, uses: int}`.
`InvitePreview`: `{code, server: {id, name, icon_url: nullable, member_count: int}, expires_at: nullable ts, member: bool}`.

| Method & path | Web fn | Request | Success | Errors |
|---|---|---|---|---|
| `GET /api/servers/{id}/invites` | `listInvites` (79) | — | `200 Invite[]` — only usable ones (not expired, uses < max), newest first | `manage_server` |
| `POST /api/servers/{id}/invites` | `createInvite` (82) | `{max_uses?: 1..=10000, expires_in_hours?: 1..=8760}`; absent = unlimited / never | `201 Invite` | any member; `422 max_uses: invalid`, `expires_in_hours: invalid` |
| `DELETE /api/invites/{code}` | `revokeInvite` (85) | — | `204` | creator, else `manage_server` |
| `GET /api/invites/{code}` | `previewInvite` (88) | — (needs session) | `200 InvitePreview` | `401` (anonymous → web sends to login with redirect back), `403 banned`, `410 invite_invalid` (only for non-members), `404` |
| `POST /api/invites/{code}/join` | `joinInvite` (91) | none | `200 ServerView` (also when already a member; then `uses` is not incremented) | `403 banned`, `410 invite_invalid`, `404` |

From v0.10 both invite routes answer `410 invite_invalid` for an invite of a blocked server (§4.5), to members as well; the preview checks this before `banned`.

Web invite terms (`web/src/servers/inviteTerms.ts:6-56`): expiry options `1 Stunde` (1 h), `24 Stunden` (24), `7 Tage` (168), `Nie` (omit); uses `1`, `5`, `25`, `Unbegrenzt` (omit); default 7 Tage / Unbegrenzt; summary text `gültig 7 Tage · unbegrenzt nutzbar` (pattern `gültig ${label}` or `unbegrenzt gültig`, then `${n}-mal nutzbar` or `unbegrenzt nutzbar`).

### 4.5 Blocked servers and reports (v0.10, `operator/servers.rs`, `operator/reports.rs`)

An instance operator can block a server. While it is blocked:

- `GET /api/servers` still lists it, with `blocked: true`, so the client can explain the block; the server opens to a notice ("Dieser Server wurde vom Betreiber gesperrt."), with "Server verlassen" for members and "Server löschen" for the owner.
- Every other route of the server, its channels and messages answers `403 server_blocked` for every member, the owner included: `GET /api/servers/{id}`, message history and writes, media tickets (§11), invites, bans, notification settings and the rest. Only `POST /api/servers/{id}/leave` and `DELETE /api/servers/{id}` work as before.
- `GET /api/invites/{code}` and `POST /api/invites/{code}/join` of its invites answer `410 invite_invalid`, to members too.
- `GET /api/messages/unread` and the read-all routes leave its channels out.
- Open sockets get `{"op":"err","e":"server_blocked","s":S}` and lose their voice seat in it (protocol-gateway §3.1).

Unblocking restores everything; the client sees it on its next `GET /api/servers`. No event announces it.

Reporting a server:

| Method & path | Request | Success | Errors |
|---|---|---|---|
| `POST /api/servers/{id}/reports` | `{"reason": "spam" \| "harassment" \| "sexual" \| "illegal" \| "other", "details"?: string}`; `details` trimmed, ≤ 1000 chars, line breaks and tabs allowed, required for `other` | `201 {"id", "created_at"}`; when the caller already has an open report of this server: `200` with that one | `404` neither a member nor able to see its listing (or unknown/unparseable id); `403 server_blocked` (members); `429 rate_limited` (`report`, 10 per hour per account, counted before validation); `422` fields `reason: invalid`, `details: too_long \| invalid \| required` |

Members can report, and while the directory is on (§4.7) so can anyone who can see the server's listing; that is how "Melden" on a listing reports. When the caller can see the listing, members included, the report stores a snapshot of it (`name`, `icon_url`, `description`, `language`, `join_mode`, `member_count` at that moment), so later edits cannot hide what was reported. Reports are read only by operators; owners never see them.

### 4.6 Operator area "Betrieb" (v0.10, web only)

`/api/operator/*` serves the web pages under `/betrieb`. A client without a session gets `401`, an account that is not an operator `404` on every one of these routes. The native app has no operator pages: for an operator (`SessionResponse.operator`) its user menu entry "Betrieb" opens `<instance origin>/betrieb/<page>` in the system browser, with `page` one of `einladungen`, `konten`, `server`, `meldungen`, `auslastung`.

| Method & path | Request | Success |
|---|---|---|
| `GET /api/operator` | — | `200 {"registration": "open" \| "invitation", "mail": bool, "open_reports": int}` |
| `GET /api/operator/invitations` | `?status=open\|used\|all&offset=` | `200 {"invitations": Invitation[], "total": int}` (50 per page) |
| `POST /api/operator/invitations` | `{"email"?, "note"?, "expires_in_days"?: 1\|7\|30\|null, "send_mail"?: bool}` | `201 Invitation` |
| `DELETE /api/operator/invitations/{id}` | — | `204` |
| `GET /api/operator/accounts` | `?q=&filter=all\|disabled\|pending&offset=` | `200 {"accounts": Account[], "total": int}` |
| `POST /api/operator/accounts/{id}/disable` | `{"reason"?: string}` (send `{}` without one) | `200 Account`; the account's sessions, sockets and media tickets end |
| `POST /api/operator/accounts/{id}/enable` | — | `200 Account` |
| `GET /api/operator/servers` | `?q=&filter=all\|listed\|reported\|blocked&offset=` | `200 {"servers": ServerAdmin[], "total": int}` |
| `POST /api/operator/servers/{id}/delist`, `/relist`, `/block`, `/unblock` | delist `{"reason"?}` (send `{}` without one), block `{"reason"}` (1..=500, required); relist and unblock none | `200 ServerAdmin` |
| `GET /api/operator/reports` | `?status=open\|closed&offset=` | `200 {"reports": Report[], "total": int}` |
| `POST /api/operator/reports/{id}/resolve` | `{"status": "dismissed" \| "actioned", "note"?}` | `200 Report` |
| `GET /api/operator/usage` | — | `200` counts of accounts, servers, messages, storage and live use |

The shapes and every error are in `backend/api/src/operator/` and issue #517. Every successful call writes a row of the operator audit (deploy/README.md, "Betrieb"), except `GET /api/operator` and disabling an account that is disabled already. A call that changes nothing else records `"changed": false` in the row's `detail`.

### 4.7 Directory "Entdecken" (v0.10, `backend/api/src/directory/`)

Off unless the instance sets `API_DIRECTORY=true` (deploy/README.md, "Verzeichnis"). Signed-in only: every route answers `401` without a session, also while the directory is off. While it is off, `GET /api/directory` answers `{"enabled": false, "languages": []}`, and every `/api/directory/listings*` route and `/api/servers/{id}/listing` answer `404`; the queue routes keep working.

A listing is **visible** to the caller while the owner has it `listed`, the server is neither delisted nor blocked by an operator (§4.5, §4.6), and the caller is not banned from it. A listing the caller cannot see answers `404` on every route, like an unknown id.

Shapes:

- `Language`: `{"code": "de", "name": "Deutsch", "listings": 3}`; `name` is the endonym and is not translated; `listings` counts the listings visible to the caller. The codes, in this order: `de`, `en`, `es`, `fr`, `it`, `nl`, `pl`, `pt`, `tr`, `uk`.
- `Listing`: exactly `{"server_id", "name", "icon_url": string|null, "description", "language", "member_count": int, "join_mode": "open"|"request", "member": bool, "request": null|"pending"|"declined"}`. `member` and `request` are the caller's own; `request` is `"declined"` only while its 7-day cooldown lasts. `icon_url` loads through `GET /api/server-icons/…`, which any signed-in account may read.
- `ListingSettings`: `{"listed": bool, "description": string, "language": string|null, "join_mode": "open"|"request", "delisted": bool, "listed_at": string|null}`. Without a saved form: `listed: false`, `description: ""`, `language: null`, `join_mode: "request"`, `listed_at: null`. `delisted` says an operator took the server out; the operator's reason is not shown.
- `JoinRequest`: `{"user_id", "name", "avatar_url": string|null, "message", "created_at"}`.

| Method & path | Who | Request | Success | Errors |
|---|---|---|---|---|
| `GET /api/directory` | any account | — | `200 {"enabled": bool, "languages": Language[]}`; all ten languages, also those with `listings: 0` | — |
| `GET /api/directory/listings` | any account | `?q=&language=&offset=`: `q` trimmed, ≤ 100 chars, no control characters, blank = no search, matches name or description case-insensitively with `%` and `_` literal; `language` a code from the list, empty = all; `offset` a multiple of 30 from 0 to 990 | `200 {"listings": Listing[], "next_offset": int\|null}`, 30 per page, ordered by `member_count` descending, then first listed, then server id; `next_offset` is `null` on the last page | `422` fields `q: too_long \| invalid`, `language: invalid`, `offset: invalid` |
| `POST /api/directory/listings/{id}/join` | any account | none | `200 ServerView` (§4.1, as `POST /api/invites/{code}/join`); also when already a member | `404` not visible; `409 join_request_required` (join mode `request`, not a member); `429 rate_limited` (`dir_join`, counted before anything else, also for a member) |
| `POST /api/directory/listings/{id}/request` | any account | `{"message"?: string}`: trimmed, one line, ≤ 300 chars; absent = empty | `201 {"request": "pending"}` new; `200 {"request": "pending"}` when one is pending already (its message is kept) | in this order: `422` field `message: too_long \| invalid` (before the limit); `429 rate_limited` (`dir_join`); `404` not visible; `409 already_member`; `409 join_request_declined` with `retry_after` while the cooldown lasts; `409 join_requests_full` when the caller has 10 pending requests or the server 100 |
| `DELETE /api/directory/listings/{id}/request` | the requester | — | `204` | `404` no pending request. No visibility check, so it works after a delist |
| `GET /api/servers/{id}/listing` | any member | — | `200 ListingSettings` | `404` not a member; `403 server_blocked` |
| `PUT /api/servers/{id}/listing` | owner | `{"listed": bool, "description": string, "language": string, "join_mode": "open"\|"request"}`, all four required; `description` trimmed, one line, ≤ 300 chars, not empty while `listed` | `200 ListingSettings`; while delisted the save succeeds and the listing stays hidden (`delisted: true`) | `404` not a member; `403 forbidden` not the owner; `403 server_blocked`; `422` fields `listed: required`, `description: required \| too_long \| invalid`, `language: required \| invalid`, `join_mode: required \| invalid` |
| `GET /api/servers/{id}/join-requests` | owner, `moderate_members` | — | `200 JoinRequest[]`, pending only, oldest first, at most 100 | `404` not a member; `403 forbidden`; `403 server_blocked` |
| `POST /api/servers/{id}/join-requests/{user_id}/accept` | owner, `moderate_members` | none | `204`; the requester becomes a member (`jn` to them), or only loses the request if they are a member already | `404` no pending request, or the requester was banned meanwhile (the request is deleted); `403 forbidden`; `403 server_blocked` |
| `POST /api/servers/{id}/join-requests/{user_id}/decline` | owner, `moderate_members` | none | `204`; the requester sees `request: "declined"` for 7 days | `404` no pending request; `403 forbidden`; `403 server_blocked` |

- The queue routes work whatever the listing, the delist or `API_DIRECTORY` say, so pending requests can always be answered.
- After every new request (`201`), withdrawal, accept and decline, the owner and every member holding `moderate_members` get `jq` with the new pending count; an accepted requester gets `jn` (protocol-gateway §3). Both are best effort; the next REST read catches up.
- Joining by invite, joining through the directory and a ban each delete the person's request for that server; these send no `jq`.
- The join mode governs only the directory: invites work as before. Switching the mode or unlisting keeps the pending requests.
- Reporting a listing uses `POST /api/servers/{id}/reports` (§4.5).

---

## 5. Direct messages (`backend/api/src/dms/mod.rs`; web `web/src/dms/api.ts`, `web/src/dms/types.ts`)

`DirectMessage`: `{id: uuid (the DM channel id), kind: "dm", created_at: ts, peer: {id, name, avatar_url: nullable}}`.

| Method & path | Web fn | Request | Success | Errors |
|---|---|---|---|---|
| `GET /api/dms` | `listDms` (4) | — | `200 DirectMessage[]` ordered by newest message time (or channel creation when empty) DESC, then id | 401 |
| `GET /api/dms/{id}` | `getDm` (7) | — | `200 DirectMessage` | `404` not a participant |
| `POST /api/dms` | `openDm` (10) | `{user_id: string}` | `201` newly created, `200` existing (idempotent per pair) | `422 user_id: required\|invalid` (also self); `404` unknown user |

Messages, reads, notification settings and attachments of a DM use the channel routes below with the DM id as `{id}`; permissions do not apply in DMs (both participants may send/upload), and only the author may delete (`messages/mod.rs:713-724`).

---

## 6. Messages (`backend/api/src/messages/mod.rs`; web `web/src/messages/api.ts`, `types.ts`)

### 6.1 Shapes

`Message` (`messages/mod.rs:71-100`; web `messages/types.ts:22-53`):

| Key | Type | Presence |
|---|---|---|
| `id` | uuid | always |
| `channel_id` | uuid | always |
| `author` | `{id: uuid, name: string, avatar_url: nullable string}` | always |
| `content` | string (≤ 2000 chars, may be `""` for GIF/attachment-only/system) | always |
| `created_at` | ts | always |
| `edited_at` | nullable ts | always |
| `revision` | integer (i64) — durable delivery revision; higher = newer | always |
| `created_order` | integer (i64) — immutable insert order | always |
| `attachments` | `Attachment[]` | always (may be `[]`) |
| `reactions` | `{emoji: string, user_ids: uuid[]}[]`, sorted by emoji (text), user_ids sorted | always |
| `gif` | `Gif` (§12) | optional (absent when none) |
| `mentions` | uuid[] — users the server resolved as mentioned | always |
| `mention_everyone` | bool | always |
| `reply` | `Reply` | optional |
| `previews` | `LinkPreview[]` | optional, absent when empty |
| `system` | `{kind: string, params: object}` | optional; only kind today `live_started` with params `voice_channel_id`, `voice_name` (`messages/system.rs:1-35`) |
| `nonce` | uuid | optional; only in the create response and its live event — echo of the client's attempt id |

`Attachment` (`attachments/mod.rs:63-69`): `{id, filename, content_type, size: integer bytes}`.

`Reply` (`messages/replies.rs:20-44`): `{id, deleted: bool, author?: {id, name}, excerpt?: string (≤ 100 chars + "…", spoilers `||x||` → `▒▒▒`), has_attachment?: true, has_gif?: true, system?: SystemMessage}`; a deleted/foreign original is exactly `{id, deleted: true}`. `has_attachment`/`has_gif` are omitted when false. `reply_to` is stored unchecked; an id outside the channel renders as deleted.

`LinkPreview` (`previews/mod.rs:69-91`): `{url: string, title?, description?, site_name?, image?: {url: "/api/previews/{id}/{file}", width, height}, youtube_id?: 11 chars}`. Web keeps only http(s) `url`, images under `/api/previews/` with positive size, and ids matching `^[A-Za-z0-9_-]{11}$` (`messages/types.ts:145-174`).

`MessagePage` (`messages/mod.rs:110-114`): `{messages: Message[] (oldest → newest), has_more: bool}`. The web adds a client-only `older` cursor (`messages/pages.ts:153-157`); the server never sends it.

Wire-shape caveat: live events re-encoded through Redis Lua cjson may turn `[]` into `{}` (`messages/types.ts:143,216,232-248`). REST responses are plain serde and never do this, but a native client sharing one decoder for REST and `/ws` should accept `{}` as an empty list for `attachments`, `reactions`, `mentions`, `previews`.

### 6.2 History and pagination

`GET /api/channels/{id}/messages?before=&after=&limit=` (`messages/mod.rs:242-263, 809-894`; `messages/validate.rs`)

| Param | Accepted forms | Meaning |
|---|---|---|
| `before` | `"{created_at}\|{uuid}"` (preferred), a message uuid of this channel, or an RFC 3339 time | page of messages strictly older than the bound |
| `after` | same | page strictly newer than the bound |
| `limit` | 1..=100, default 50 | page size |

- `before` and `after` together → `422 before: invalid`. Unparseable cursor → `422 {field}: invalid`; a uuid cursor not in the channel → `422 {field}: invalid`.
- No cursor: newest `limit` rows. `has_more` = more older rows exist.
- `before`: rows `(created_at, id) < bound` ordered DESC, then reversed; `has_more` = more older rows.
- `after`: rows `> bound` ASC; `has_more` = more newer rows.
- Response always oldest → newest. Attachments, replies and previews are filled in.
- `404` for voice channels, unknown channel, non-member.

Web usage (`messages/queries.ts:336-347`, `pages.ts:147-182`, `rules.ts:7`): first page `?limit=50`; older pages `?before={oldest.created_at}|{oldest.id}&limit=50` while `has_more`. **Encode query values** (`URLSearchParams`): the cursor contains `:`/`|` and, for context cursors, `+` which must become `%2B`.

### 6.3 Write routes

| Method & path | Web fn | Request | Success | Errors |
|---|---|---|---|---|
| `POST /api/channels/{id}/messages` | `createMessage` (`messages/api.ts:29-46`) | Text: `{content, attachment_ids: uuid[], reply_to?: uuid, nonce?: uuid}`. GIF: `{content: "", gif: {slug, query}, reply_to?, nonce?}` (no `attachment_ids`). | `201 Message` (with `nonce` echoed) | `send_messages` (server); `send_files` when attachments; `422`: `content: required` (nothing to send) / `too_long` (> 2000) / `invalid` (control chars other than `\n`,`\t`); `attachment_ids: invalid` (duplicate, unknown, not yours, other channel, expired) / `too_long` (> 10) / `too_large` (sum > 100 MiB); `attachment_ids.N: mismatch` (bytes don't match type); `size: invalid`; `gif: exclusive\|unavailable\|invalid`; `429` msg bucket, `gif` bucket |
| `PATCH /api/messages/{id}` | `updateMessage` (48) | `{content}` | `200 Message` (unchanged content returns current) | author only (`403 You can only edit your own messages.`); `send_messages`; `422 gif: not_editable`, `system: not_editable`, `content …` |
| `DELETE /api/messages/{id}` | `deleteMessage` (51) | — | `204` (also if already gone) | author, or `manage_messages` on a server; in a DM non-author → `403 You can only delete your own messages.` |
| `DELETE /api/messages/{id}/previews` | `removePreviews` (55) | — | `204`; previews removed for good (`suppress_embeds`) | author or `manage_messages` |
| `PUT /api/messages/{id}/reactions/{emoji}` | `messages/reactions.ts:71-76` | — ; `{emoji}` = `encodeURIComponent(emoji)` | `200 Message` (full, updated) | add needs `send_messages`; `422 emoji: invalid` (not in Unicode 18 emoji list, FE0F-insensitive) / `emoji: limit` (20 distinct emoji per message); idempotent |
| `DELETE /api/messages/{id}/reactions/{emoji}` | same | — | `200 Message` | idempotent |

Content normalisation (server): `\r\n` and `\r` → `\n`, then trim. Optimistic UI: the web inserts a `tmp:<nonce>` row and replaces it by the response or the live event carrying the same `nonce`. Reaction/edit responses should be merged only if `revision` ≥ cached revision (`messages/reactions.ts:78-90`).

### 6.4 Search and context (`messages/workflows.rs:438-545`; web `messages/search.ts`)

| Method & path | Request | Success | Errors |
|---|---|---|---|
| `GET /api/channels/{id}/messages/search?q=&limit=&before=` | `q` trimmed 1..=200 chars (PostgreSQL `websearch_to_tsquery('simple', q)`); web sends `limit=25`; next page `before={oldest.created_at}\|{oldest.id}` while `has_more` | `200 MessagePage` (oldest → newest within the page, newest hits first page) | `422 q: required\|too_long\|invalid`, `limit`, `before`, all in the JSON envelope (1.5). A missing `q` is `q: required`, not a plain 400; `invalid` is a control character in `q` |
| `GET /api/channels/{id}/messages/{message_id}/context` | — | `200 {target_id: uuid, messages: Message[] (≤ 30 older + target + ≤ 30 newer, oldest → newest), before: "ts\|uuid", after: "ts\|uuid"}` — use `before`/`after` as cursors for further history | `404` message not in channel |

---

## 7. Read state and unread counts (`messages/workflows.rs:42-436`; web `messages/readState.ts:20-60`)

`ReadState`:

| Key | Type |
|---|---|
| `channel_id` | uuid (text channel or DM) |
| `server_id` | nullable uuid (null for DMs) |
| `read_message_id` | nullable uuid |
| `read_at` | nullable ts |
| `read_order` | nullable int (created_order of the read message) |
| `membership_at` | ts (join time the position belongs to) |
| `latest_order` | nullable int (newest message's created_order) |
| `unread_count` | int, capped at 100 (`LIMIT 100`, `messages/workflows.rs:97-101`; count of others' messages after the read position, since joining) — the UI should render ≥ 100 as a capped badge |
| `mention_count` | int; unread messages that mention the user (`<@id>`, `@here` if they were online then, `@everyone`), counted among the newest 1000 unread mentioning messages (`messages/workflows.rs:55-59, 107-113`) |
| `level` | `"all"` \| `"mentions"` \| `"none"` — effective notification level (channel → server → `all`) |
| `muted` | bool — channel or its server muted now |
| `muted_until` | nullable ts — null while muted indefinitely or not muted |

| Method & path | Web fn | Request | Success | Notes |
|---|---|---|---|---|
| `GET /api/messages/unread` | `listReadState` | — | `200 ReadState[]` for every text channel of every server + every DM | |
| `PUT /api/channels/{id}/read` | `markChannelRead` / `markRead` | `{}` (to newest since joining) or `{message_id}`; JSON body required | `200 ReadState` | position only moves forward; `404` message not in channel / voice channel |
| `PUT /api/servers/{id}/read` | `markServerRead` | no body | `200 ReadState[]` (only rows that moved) | `read_bulk` limit |
| `PUT /api/messages/read-all` | `markAllRead` | `{scope: "all" \| "dms"}` (default `all`) | `200 ReadState[]` (moved rows) | `read_bulk` limit |

Moves are pushed to the account's other sockets as a `read` user frame (gateway spec).

---

## 8. Notification settings and privacy

### 8.1 Settings (`notifications/settings.rs`; web `messages/notificationSettings.ts:25-56`)

`Setting`: `{scope: "server" | "channel", id: uuid, level: "all"|"mentions"|"none"|null, muted: bool, muted_until: nullable ts}`. Only explicit settings are listed (ordered by scope, id); an absent row means inherit (`level: null`, not muted).

| Method & path | Request | Success | Errors |
|---|---|---|---|
| `GET /api/notification-settings` | — | `200 Setting[]` | 401 |
| `PUT /api/servers/{id}/notification-settings` | `{level: Level\|null, muted: bool, mute_minutes?: 1..=527040}` | `200 Setting` | `404` non-member; `422 mute_minutes: invalid` (given while `muted` false, or out of range) |
| `PUT /api/channels/{id}/notification-settings` | same; `{id}` = text channel or DM | `200 Setting` | `404` voice/foreign |

`level: null` + `muted: false` deletes the row (inherit). `muted_until` = server `now() + mute_minutes`; absent `mute_minutes` with `muted: true` = until turned back on. Other sockets receive an `ns` frame and refetch.

### 8.2 Privacy (`notifications/privacy.rs`; web `messages/notificationPrivacy.ts:21-33`)

| Method & path | Request | Success |
|---|---|---|
| `GET /api/notification-privacy` | — | `200 {"hide_content": bool}` |
| `PUT /api/notification-privacy` | `{"hide_content": bool}` (required; missing → 400) | `200 {"hide_content": bool}` |

---

## 9. Attachments (`backend/api/src/attachments/mod.rs`, `validate.rs`; web `messages/api.ts:58-120`)

Limits (`attachments/validate.rs:7-35`; web `messages/rules.ts:7-14`): file ≤ 25 MiB (26 214 400 B); ≤ 10 files per message; ≤ 100 MiB per message; filename ≤ 255 chars after stripping any path (`/`, `\`), no control chars; allowed types: `image/jpeg, image/png, image/gif, image/webp, application/pdf, text/plain, text/markdown, text/csv, application/json, application/zip, application/gzip, application/x-tar, application/x-7z-compressed, audio/mpeg, audio/wav, audio/ogg, audio/flac, audio/mp4, audio/aac, audio/webm, video/mp4, video/webm, video/quicktime` (aliases canonicalised: `image/jpg`, `image/pjpeg` → jpeg; `audio/mp3`, `audio/x-mpeg` → mpeg; `audio/x-wav`, `audio/wave`, `audio/vnd.wave` → wav; `audio/x-flac` → flac; `audio/x-m4a`, `audio/m4a` → mp4; `audio/x-aac` → aac; `application/x-zip-compressed`, `application/x-zip` → zip; `application/x-gzip`, `application/x-tgz` → gzip; `text/x-markdown` → markdown; parameters after `;` dropped, lower-cased). Inline types (shown in the page): `image/*` of the four above, `video/*`, `audio/*`. Text preview in the chat for files ≤ 100 KiB (`rules.ts:14`).

### 9.1 Flow (web `messages/queries.ts` send path, files uploaded one after another)

1. `POST /api/channels/{id}/attachments` with `{filename, content_type, size}` (`presignAttachment`, `api.ts:58-62`).
   Response `201`:
   ```json
   {"id":"uuid","upload_url":"https://minio…/att/uuid?X-Amz-…","headers":{"Content-Type":"image/png","Content-Length":"12345"},"expires_in":600,"attachment":{"id":"uuid","filename":"a.png","content_type":"image/png","size":12345}}
   ```
   - `ATTACHMENT_DELIVERY=redirect` (default): `upload_url` is a presigned MinIO URL on another origin, valid 600 s (`storage.rs:23`), `headers` = `Content-Type`, `Content-Length`.
   - `ATTACHMENT_DELIVERY=proxy`: `upload_url = "/api/attachments/{id}/upload"` (relative, same origin) and `headers` additionally contains `X-CSRF-Token` copied from the presign request (`attachments/mod.rs:157-173`).
   Errors: `send_files`; `422 filename: required|too_long|invalid`, `content_type: required|invalid`, `size: required (≤0)|too_long (>25 MiB)`; `429` up bucket / `quota_exceeded` (bytes are reserved now).
   The pending upload expires after 1 h (`PENDING_TTL`).
2. `PUT {upload_url}` with the raw file bytes and **every** header from `headers` (the browser skips `Content-Length` because fetch sets it from the body; a native client must send the exact length) (`putPresigned`, `api.ts:71-109`). No cookies to MinIO. In proxy mode the request carries the session cookie and the listed `X-CSRF-Token`. Success = any 2xx (proxy answers `204`). Proxy errors: `404` (not your pending upload, older than 600 s, or proxy mode off), `422 content_type: invalid` (header ≠ presigned type), `422 size: invalid` (Content-Length or body length ≠ declared). Web error copy (`messages/api.ts:93-108`): timeout (120 s) `Der Upload hat zu lange gedauert und wurde abgebrochen.`, abort `Der Upload wurde abgebrochen.`, any other fetch failure → `ApiError("network")` = `Keine Verbindung zum Server.`, non-2xx `Der Upload ist fehlgeschlagen.` (the response body is not read).
3. `POST /api/channels/{id}/messages` with `attachment_ids` in the order they should appear. The server HEADs each object and checks size and magic bytes (`mismatch`).

### 9.2 Download

`GET /api/attachments/{id}` and `GET /api/attachments/{id}?download=1` (`attachments/mod.rs:241-321`; web `attachmentUrl`, `attachmentDownloadUrl`, `api.ts:111-120`).

- Auth: session cookie; pending uploads only for the uploader; bound ones for any member of the channel/DM; else `404`.
- `download=1` or a non-inline type → `Content-Disposition: attachment; filename…`; inline types → `inline`.
- Redirect mode: `307 Temporary Redirect` to a presigned MinIO GET URL (signed per 30-min window, valid 60 min), `Vary: Cookie`, `Cache-Control: private, max-age=<seconds left in window>` (inline) or `private, no-store` (download). Follow the redirect **without** cookies/Authorization.
- Proxy mode: `200`/`206` stream with `Content-Type` (text types served as `text/plain`), `Cache-Control: private, max-age=3600` or `private, no-store`, `Vary: Cookie`, `X-Content-Type-Options: nosniff`, `Content-Security-Policy: sandbox`, `Accept-Ranges: bytes`, `Content-Length`, `Content-Range` for ranges; `416` with `Content-Range: bytes */<size>`.
- The web reads text attachments with a plain `fetch(attachmentUrl(id))` and decodes the bytes (`components/Attachments.tsx:272-300`).
- Note: `attachmentUrl`, `attachmentDownloadUrl` (`messages/api.ts:111-120`) and `gifMediaUrl` (`gifs/api.ts:34-36`) hard-code the `/api` prefix rather than `API_BASE`; image `src` values the server returns (`avatar_url`, `banner_url`, `icon_url`, preview `image.url`) are also origin-relative `/api/…` paths. A native client resolves all of them against the instance origin.

---

## 10. Link preview images (`previews/mod.rs:155-158, 704-745`)

`GET /api/previews/{id}/{file}`: any signed-in user; `200` image (PNG/JPEG/WebP sniffed), `Cache-Control: private, max-age=31536000, immutable`, `nosniff`; `404` when the preview is gone. Previews are created asynchronously after send/edit (rate 10 messages/min per user, 60 per server) and arrive as a message edit event; there is no REST route to request one. Links to the instance's own host get none.

---

## 11. Media ticket (REST part only) (`backend/api/src/media.rs:33-90`; web `voice/media.ts:195-199`)

`POST /api/channels/{id}/media-ticket`, no body → `200 {ticket: string (12 chars of the invite alphabet), expires_in: int seconds (default 30, capped by session expiry), media_path: "/media/ws", ice_servers: [{urls: string[], username?: string, credential?: string}]}`.
Errors, in check order (`media.rs:45-67`): `404` unknown/DM/non-member channel; `401` session gone; `403 forbidden` (`message: "You need the join_voice permission."`) — checked **before** the channel kind, so a text channel without `join_voice` also answers 403; `400 bad_request` for a text channel; v0.10: `403 server_blocked` for a channel of a blocked server (§4.5). The 400 body's `message` is the generic `Request body could not be read.` — the "Media tickets are for voice channels." text is only logged (`BadRequest(detail)` never reaches the wire, `error.rs:133, 184`). `ice_servers[].username`/`credential` are omitted (not `null`) when absent (`shared/src/ice.rs:16-22`); `expires_in` default from `MEDIA_TICKET_TTL_SECS` = 30 (`config.rs:89`). The ticket is the only credential the media socket uses (web builds `ws(s)://<host>/media/ws`, `voice/media.ts:191-194`).

---

## 12. GIFs (`backend/api/src/gifs/mod.rs`; web `web/src/gifs/api.ts`)

`Gif`: `{provider: "klipy", slug: string, title: string (may be ""), width: int>0, height: int>0, url: string (Klipy animated WebP/GIF), still: nullable string (first-frame JPEG)}`.
`GifPage`: `{gifs: Gif[] (24 per page), next_page: int | null}`.

| Method & path | Web fn | Request | Success | Errors |
|---|---|---|---|---|
| `GET /api/gifs` | `gifStatus` (15) | — | `200 {"enabled": bool, "provider": "klipy" \| null}` | 401 |
| `GET /api/gifs/trending?page=` | `gifPage` with empty query (18-32) | `page` 1..=50 (web omits for page 1) | `200 GifPage` | `404` GIFs disabled; `422 page: invalid`; `429` gif bucket; `502 upstream_unavailable`; `503 upstream_quota` + `retry_after` |
| `GET /api/gifs/search?q=&page=` | `gifPage` with query | `q` trimmed 1..=100 chars, no control chars | `200 GifPage` | as trending + `422 q: required\|too_long\|invalid` |
| `GET /api/gifs/media?url=<klipy url>` | `gifMediaUrl` (34-36), used for every GIF `<img>` (picker and messages) | `url` must be on the configured Klipy media hosts | image stream (`image/webp|gif|jpeg`, ≤ 16 MiB), `Cache-Control: private, max-age=3600`, honours `If-None-Match`/`If-Modified-Since` | `404` disallowed URL or disabled; `429` gif_media bucket; `502` |

Send a GIF via `POST …/messages` with `{content: "", gif: {slug, query}}` (§6.3). Locale for Klipy comes from the request's `Accept-Language` first tag (region if present, e.g. `de-AT` → `at`, else language) — a native client should send `Accept-Language: de-DE` (or the OS locale) to match the web.

GIF box in messages: at most 320×240, never upscaled (`gifBox`, `gifs/api.ts:40-50`); alt text `GIF: ${title}` or `GIF`. Picker status copy (`gifFeedStatus`, `gifs/api.ts:63-91`), first match wins: `GIFs werden geladen …` (pending); `rate_limited` with `retry_after` → `Zu viele GIF-Anfragen. Bitte in ${retry_after} s noch einmal versuchen.`, without → `Zu viele Anfragen. Bitte kurz warten und noch einmal versuchen.`; `upstream_quota` without `retry_after` → `Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte später noch einmal versuchen.`; `upstream_unavailable` → `Der GIF-Dienst antwortet gerade nicht. Bitte gleich noch einmal versuchen.`; with `retry_after`: `Dieser Server hat sein GIF-Kontingent gerade aufgebraucht. Bitte in ${n s | n Min.} noch einmal versuchen.` (`< 60` → `${s} s`, else `${ceil(s/60)} Min.`); `GIFs konnten nicht geladen werden.`; `Weitere GIFs konnten nicht geladen werden.`; `Keine GIFs gefunden.`; `${count} GIFs zu „${query}“`; `${count} angesagte GIFs`.

---

## 13. Web Push (browser only) (`push/mod.rs:144-249`; web `pwa/push.ts:30-66`)

| Method & path | Request | Success |
|---|---|---|
| `GET /api/push/key` | — | `200 {"key": string \| null}` (VAPID public key, base64url) |
| `POST /api/push/subscriptions` | `{endpoint: string ≤ 1024 on an allowed push host, keys: {p256dh, auth}}` | `204` (max 20 per user, bound to the session) — `404` push disabled, `422 endpoint|keys: invalid` |
| `DELETE /api/push/subscriptions` | `{endpoint}` | `204` |

A native desktop client has no browser push endpoint; it relies on `/ws` while running. Not needed for parity.

---

## 14. Startup request set of the web client (for parity)

After `authenticated` the shell (`components/AppShell.tsx:28-57`, `pages/WorkspaceLayout.tsx:61-264`) issues, in parallel: `GET /api/servers`, `GET /api/dms`, `GET /api/messages/unread`, `GET /api/notification-settings`, `GET /api/notification-privacy` (via the toast/live-notice bridges, `messages/useMessageToasts.ts`, `voice/useLiveNotices.ts`), `GET /api/me/themes`, the `/ws` upgrade. `GET /api/push/key` + `POST /api/push/subscriptions` only run when the browser's `Notification.permission === "granted"` and a service worker is active (`pwa/push.ts:34-44`); a native client skips both. Opening a server: `GET /api/servers/{id}`. Opening a channel/DM: `GET …/messages?limit=50` then `PUT …/read`. GIF picker open: `GET /api/gifs`, then trending. Login page: `GET /api/auth/mail`.

---

## 15. Native client (Rust, reqwest) — cookie mode against the unchanged backend

Everything below works with today's backend; no bearer auth exists yet (planned as B1 in `docs/v0.9-plan.md:324`).

### 15.1 Required

| # | Requirement | Why / source |
|---|---|---|
| 1 | Configure an instance origin, e.g. `https://gelabber.example`; all REST under `{origin}/api`, gateway `{ws(s)}://{host}/ws`, media `{ws(s)}://{host}{media_path}` | web uses same-origin relative paths |
| 2 | Keep a cookie jar (reqwest `cookie_store`, persisted to disk to survive restarts) for `gelabber_session` and `gelabber_csrf`. Respect `Secure`: with `API_COOKIE_SECURE=true` the cookies are only sent over https. | §1.3 |
| 3 | First call `GET /api/auth/session`; store `csrf_token` in memory; the jar keeps the matching cookie. | `auth/mod.rs:555-575` |
| 4 | Send `X-CSRF-Token: <token>` on every `POST/PUT/PATCH/DELETE` under `/api` — login and register included. The header must equal the `gelabber_csrf` cookie the jar sends. | `csrf.rs:40-59` |
| 5 | After any response body with `csrf_token` (session, login, register, logout, logout-all, password, verify-email, password-reset/confirm), replace the stored token; the response's `Set-Cookie` replaces the cookie. | rotation, `auth/mod.rs:325-379` |
| 6 | On `403 csrf_invalid` for a mutation: `GET /api/auth/session`, adopt token, retry once (abort if the returned user id differs from the one that started the mutation). | `client.ts:498-563` |
| 7 | Send `Content-Type: application/json` for every JSON body, including empty `{}` bodies (e.g. `PUT /api/channels/{id}/read`). Routes without a body extractor (`logout`, `logout-all`, `leave`, `join`, `media-ticket`, `servers/{id}/read`, deletes) accept an empty body. | `json.rs:28-38` |
| 8 | Sign-in over plain HTTP to a non-loopback host fails with `403 insecure_transport` unless the server sets `API_ALLOW_PLAIN_HTTP=true` (or runs TLS with `API_COOKIE_SECURE=true`). `http://localhost`/`127.0.0.1` always works. | `auth/mod.rs:540-551`, `deploy/README.md:20` |
| 9 | Do not send `Sec-Fetch-Site: cross-site` and do not send `X-Forwarded-For`. | `csrf.rs:41-47`, `limits.rs:500-508` |
| 10 | Treat any `401 unauthenticated` as signed out (clear user caches, show login). | `client.ts:478-484` |
| 11 | Image/attachment/GIF GETs need the session cookie (`/api/avatars`, `/api/banners`, `/api/server-icons`, `/api/previews`, `/api/attachments`, `/api/gifs/media`). Follow the `307` from `/api/attachments/{id}` to MinIO; reqwest strips `Cookie`/`Authorization` on cross-host redirects, and the jar has no cookies for the MinIO host — verify (planned test, `docs/v0.9-plan.md:297,421`). Avatars may also be external `https://` URLs: fetch them without cookies and without `Referer`. | §3.2, §9.2 |
| 12 | Timeouts: 10 s per REST call, 120 s for upload PUTs; track `Date` for server time. | `client.ts:313, 591-604` |
| 13 | Encode query parameters (cursors contain `|`, `:` and possibly `+`). | §6.2 |

### 15.2 Optional / not required

| Item | Status |
|---|---|
| `Origin` header | Not checked on REST at all. On the `/ws` upgrade it is optional: absent → allowed; present → must be `http(s)://` + the request's `Host` (default ports ignored), `null` and non-ASCII refused, and with `API_COOKIE_SECURE=true` an `http://` origin is refused unless loopback (`gateway/mod.rs:75-117`). tokio-tungstenite sends no `Origin` by default — leave it out. |
| `Referer`, `Sec-Fetch-*`, `User-Agent`, `Accept` | ignored by the API |
| CSRF token format | the check only compares cookie and header; a client could set its own pair, but should use the server-issued token so `/api/auth/session` does not replace the cookie |
| `Accept-Language` | only affects Klipy locale |

### 15.3 Request example (login from scratch)

```
GET /api/auth/session HTTP/1.1
Host: gelabber.example
Accept: application/json

→ 200
Set-Cookie: gelabber_csrf=9f…64hex; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax; Secure
{"user":null,"csrf_token":"9f…64hex"}

POST /api/auth/login HTTP/1.1
Host: gelabber.example
Accept: application/json
Content-Type: application/json
Cookie: gelabber_csrf=9f…
X-CSRF-Token: 9f…

{"email":"ada@example.com","password":"…"}

→ 200
Set-Cookie: gelabber_session=ab…; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax; Secure
Set-Cookie: gelabber_csrf=c3…; Path=/; Max-Age=31536000; HttpOnly; SameSite=Lax; Secure
{"user":{"id":"…","email":"ada@example.com","name":"Ada","avatar_url":null,"created_at":"2026-10-01T09:00:00.123456Z","bio":null,"pronouns":null,"status_text":null,"accent_color":null,"banner_color":null,"banner_url":null,"avatar_decoration":null},"csrf_token":"c3…"}
```

---

## 16. Full route index (backend) with web usage

| Method | Path | Web uses | § |
|---|---|---|---|
| GET | /api/auth/session | yes | 2 |
| GET | /api/auth/mail | yes | 2 |
| GET | /api/auth/registration | yes (v0.10) | 2 |
| POST | /api/auth/register, /login, /logout, /logout-all, /password, /verify-email, /password-reset, /password-reset/confirm | yes | 2 |
| GET, PATCH | /api/me | PATCH only | 3 |
| GET | /api/users/{id} | yes | 3 |
| PUT | /api/me/avatar, /api/me/banner | yes | 3 |
| GET | /api/avatars/{user}/{file}, /api/banners/{user}/{file} | img | 3 |
| GET, PUT | /api/me/themes | yes | 3.3 |
| GET, POST | /api/servers | yes | 4 |
| GET, PATCH, DELETE | /api/servers/{id} | yes | 4 |
| POST | /api/servers/{id}/leave | yes | 4 |
| PUT, DELETE | /api/servers/{id}/icon | yes | 4 |
| GET | /api/server-icons/{server}/{file} | img | 4 |
| POST | /api/servers/{id}/categories | yes | 4 |
| PATCH, DELETE | /api/categories/{id} | yes | 4 |
| POST | /api/servers/{id}/channels | yes | 4 |
| PATCH, DELETE | /api/channels/{id} | yes | 4 |
| GET, POST | /api/servers/{id}/invites | yes | 4.4 |
| GET, DELETE | /api/invites/{code} | yes | 4.4 |
| POST | /api/invites/{code}/join | yes | 4.4 |
| POST | /api/servers/{id}/kick, /api/servers/{id}/ban | yes | 4.3 |
| GET | /api/servers/{id}/bans | yes | 4.3 |
| DELETE | /api/servers/{id}/bans/{user_id} | yes | 4.3 |
| GET, POST | /api/dms | yes | 5 |
| GET | /api/dms/{id} | yes | 5 |
| GET, POST | /api/channels/{id}/messages | yes | 6 |
| PATCH, DELETE | /api/messages/{id} | yes | 6 |
| DELETE | /api/messages/{id}/previews | yes | 6 |
| PUT, DELETE | /api/messages/{id}/reactions/{emoji} | yes | 6 |
| GET | /api/channels/{id}/messages/search | yes | 6.4 |
| GET | /api/channels/{id}/messages/{message_id}/context | yes | 6.4 |
| GET | /api/messages/unread | yes | 7 |
| PUT | /api/channels/{id}/read, /api/servers/{id}/read, /api/messages/read-all | yes | 7 |
| GET | /api/notification-settings | yes | 8 |
| PUT | /api/servers/{id}/notification-settings, /api/channels/{id}/notification-settings | yes | 8 |
| GET, PUT | /api/notification-privacy | yes | 8 |
| POST | /api/channels/{id}/attachments | yes | 9 |
| GET | /api/attachments/{id} | img/fetch | 9 |
| PUT | /api/attachments/{id}/upload | proxy mode | 9 |
| GET | /api/previews/{id}/{file} | img | 10 |
| POST | /api/channels/{id}/media-ticket | yes | 11 |
| GET | /api/gifs, /api/gifs/trending, /api/gifs/search, /api/gifs/media | yes | 12 |
| GET | /api/push/key; POST, DELETE /api/push/subscriptions | browser | 13 |
| POST | /api/servers/{id}/reports | yes (v0.10) | 4.5 |
| GET, POST, DELETE | /api/operator, /api/operator/* | operator pages only (v0.10) | 4.6 |
| GET | /api/directory, /api/directory/listings | yes (v0.10) | 4.7 |
| POST | /api/directory/listings/{id}/join | yes (v0.10) | 4.7 |
| POST, DELETE | /api/directory/listings/{id}/request | yes (v0.10) | 4.7 |
| GET, PUT | /api/servers/{id}/listing | yes (v0.10) | 4.7 |
| GET | /api/servers/{id}/join-requests | yes (v0.10) | 4.7 |
| POST | /api/servers/{id}/join-requests/{user_id}/accept, /decline | yes (v0.10) | 4.7 |
| GET | /health, /ready, /metrics | no | — |
| GET | /ws (upgrade) | yes | gateway spec |

## 17. Gaps and uncertainties

- `transport_allowed` passes when the request has no `Host` header (`auth/mod.rs:547-550`). Over HTTP/2, hyper may expose the authority without a `Host` header, which would let plain-HTTP sign-in through. Not verified; do not rely on it.
- Plain-text (non-envelope) error bodies: 413 from `DefaultBodyLimit` on raw-byte routes and axum's default 404/405 for unknown routes and methods. The client must map them by status code. Routes that take a JSON `Body<T>` turn an over-limit body into the envelope `400 bad_request` instead (`json.rs:28-40`, `BytesRejection`), e.g. `PUT /api/me/themes` > 64 KiB.
- `ApiError::BadRequest(detail)` and `Forbidden(msg)` differ: a 400's `message` is always the generic `Request body could not be read.` (detail logged only), a 403 `forbidden` carries its per-call English text. The UI never shows `message`; it maps `error` to German.
- Bearer auth, per-device session lists and `/ws` versioning do not exist yet (`docs/v0.9-plan.md:97-100, 324-329`). Cookie mode is the only option today.
- Live gateway frames (`/ws`, `read`, `ns`, message `C/E/D` events whose delta is the same `Message` JSON) are covered by the gateway protocol spec, not here.
