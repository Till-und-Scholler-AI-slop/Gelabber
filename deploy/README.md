# Deploy

One operator, one node. Stack: Caddy, Postgres, Redis, MinIO, coturn, api, web, media. No LiveKit.

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up -d
```

App: `http://localhost` (Caddy :80). Data and MinIO on `127.0.0.1` only. Set the passwords in `.env` (see Security below).

## Security

Since v0.6 (#188) the stack no longer starts with known values:

- **No default passwords.** `POSTGRES_PASSWORD`, `MINIO_ROOT_PASSWORD`, `MINIO_APP_PASSWORD` and `TURN_PASSWORD` (plus `GRAFANA_ADMIN_PASSWORD` with observability) must be set in `.env`; otherwise `docker compose` stops with a message. Generate one with `openssl rand -hex 24`. `POSTGRES_PASSWORD` only applies when the database volume is created; an existing volume keeps its password until you change it with `ALTER USER gelabber PASSWORD '…'`.
- **With `API_COOKIE_SECURE=true`** (that is, behind TLS) the API refuses to start while one of these passwords is still a former repository default (`gelabber`, `gelabbergelabber`, `gelabberturn`, `change-me…`). Change the password first (Postgres as above, MinIO and TURN in `.env`), then restart.
- **The API has no MinIO root access** (#196). The one-shot service `minio-init` signs in as root, creates the bucket and the user `MINIO_APP_USER` (default `gelabber-api`) with a policy for `MINIO_BUCKET` only (read, write, delete objects; no other bucket, no admin API), and exits. The API gets only that user (`MINIO_ACCESS_KEY` / `MINIO_SECRET_KEY` in the container); the root credentials stay in `minio` and `minio-init`. A leaked API environment therefore exposes the attachments, not the whole MinIO instance. Upgrading an existing install: see "MinIO user" below.
- **Plain HTTP on this machine only.** The bundled Caddy listens on `127.0.0.1` by default. Without `API_COOKIE_SECURE=true` the API signs in only on `localhost`/`127.0.0.1`; other machines get `insecure_transport`, because the session cookie would otherwise cross the network unencrypted. For other machines put TLS in front (your own Caddy, see below) and set `API_COOKIE_SECURE=true`. For a trusted test network only: `API_ALLOW_PLAIN_HTTP=true`. In this mode browser tabs do not coordinate sign-in and sign-out with each other, and browsers allow microphone, camera and screen share only in a secure context, so voice and video in the browser need TLS (the desktop app uses native media).
- **HSTS.** With `API_COOKIE_SECURE=true` the API sends `Strict-Transport-Security: max-age=31536000` (without subdomains), whichever proxy terminates TLS.
- **Browser security headers.** The web image sends `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin`, `X-Frame-Options: DENY` and a `Permissions-Policy` that turns off features the app never uses (geolocation, payment, USB, serial, HID, MIDI) with every file, and a Content-Security-Policy with the app shell (see "Content-Security-Policy" below). Camera, microphone, screen capture and the YouTube embed are not restricted.
- **Sign-in protection.** Besides 20 attempts per minute and address, the API locks an account for 15 minutes after 10 failed attempts (`API_RATE_LOGIN_FAILURES_PER_ACCOUNT`, `0` turns it off). The address is the client's as the Caddy next to the API resolved it, with IPv6 counted per /64. Behind a proxy of your own, the bundled Caddy takes it from the proxies in `GELABBER_TRUSTED_PROXIES` (default `private_ranges`, see "Client addresses" below). The API believes the `X-Forwarded-For` header from whoever reaches it, so never publish the API port (the homelab overlay's `GELABBER_API_BIND`) beyond loopback or a network only your proxy can reach.

## Existing Caddy

UDP (TURN + SFU ICE 10000) does not go through Caddy; the host/router ports stay open.

**Small diff:** bundled Caddy on loopback, your Caddy in front of it.

```bash
# deploy/compose/.env
GELABBER_HTTP_BIND=127.0.0.1
GELABBER_HTTP_PORT=8088
```

```
gelabber.example.com {
	reverse_proxy 127.0.0.1:8088 {
		header_up Host {hostport}
	}
}
```

`header_up Host {hostport}` is mandatory for the app vhost. `/ws` compares the browser's `Origin` with `Host`, port included: `{host}` cuts off a non-default port (for example `:8443` or `GELABBER_HTTP_PORT`), and every WebSocket handshake ends with 403. Without an override, Caddy passes the incoming `Host` through to **HTTP** upstreams by default; for **HTTPS** upstreams Caddy (from v2.11) sets the Host to the upstream. Explicit forwarding makes the intent clear and prevents signature/origin errors. See [reverse_proxy headers](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#headers).

**Another proxy in front (Nginx, Traefik, …).** Since v0.6 the bundled Caddy also hands `Host` on to the services with its port; up to 0.5.x it cut the port off. What reaches the stack must therefore be the browser's `Host`: the same name and, when the browser uses a non-default port, exactly that port. Nginx: `proxy_set_header Host $http_host;` (`$host` is enough only while the browser uses the default port). The scheme's default port written out is allowed (`Host: example.com:443` for HTTPS, `:80` for HTTP).

If the proxy in front instead writes a port into `Host` that the browser does not use, `/ws` answers 403 since v0.6 ("Cross-origin WebSocket is not allowed."). That happens with `proxy_set_header Host $host:$proxy_port;` (the upstream's port, 8088 above) and with `$host:$server_port` behind a port translation (443 outside, Nginx on 8443). Under 0.5.x such a configuration worked because the port was cut off. With the v0.6 Caddyfile, login and the REST API keep working, but chat no longer updates and voice does not connect. The remedy is the `Host` line on the proxy in front, not the Caddyfile.

**Client addresses.** The rate limits count per client address; sign-in, registration, email verification and password reset share 20 requests per minute and address. Behind your proxy the bundled Caddy sees only that proxy, and without help every user would share its one window. Since v0.7.1 the bundled Caddy therefore believes `X-Forwarded-For` from the peers in `GELABBER_TRUSTED_PROXIES` (`.env`; unset means `private_ranges`, the loopback and private IPv4/IPv6 ranges). It reads the header right to left, skips trusted hops and hands the API the one address it resolved.

- The default covers a proxy on the same host (it reaches the bundled Caddy through the Docker gateway) or on the LAN. The proxy in front must set or append `X-Forwarded-For`: Caddy does so by itself, Nginx needs `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;`.
- Behind a CDN or tunnel, also trust the addresses its requests arrive from, for example a CDN's published ranges: `GELABBER_TRUSTED_PROXIES=private_ranges 173.245.48.0/20 …`. A Caddy of your own in front then needs the same ranges in its global `servers { trusted_proxies static … }` options, or it replaces the header with the CDN's address; Nginx's `$proxy_add_x_forwarded_for` keeps the entry. Otherwise everybody counts as the CDN.
- `private_ranges` also trusts the clients on your LAN, and the bundled Caddy skips every trusted entry, a client's own private address included. LAN clients can therefore pick the address they are counted under when they reach the bundled Caddy directly (with a widened `GELABBER_HTTP_BIND`, or with rootless Docker and Docker Desktop, where every peer looks like the gateway) and when they come through a proxy in front that appends to `X-Forwarded-For`. Behind a proxy that overwrites the header instead, all LAN clients count as that proxy (or the gateway) and share one window. The per-account lockout and the per-address mail limit still hold. The exact address the bundled Caddy sees your proxy as avoids both (on the same host usually the gateway of the Compose network, `docker network inspect gelabber_default`).
- An empty value (`GELABBER_TRUSTED_PROXIES=`) trusts no proxy, as before v0.7.1.

**Content-Security-Policy.** Since v0.7.1 the web image sends one with the app shell. It is about scripts only: `script-src 'self' 'wasm-unsafe-eval'` plus the hash of the inline theme script (`'wasm-unsafe-eval'` lets the RNNoise noise suppression compile), `object-src 'none'`, `base-uri 'self'`, `form-action 'self'`, `frame-src https://www.youtube-nocookie.com` and `frame-ancestors 'none'`. It has no fetch directives (`default-src`, `connect-src`, `img-src`, `media-src`), so uploads and attachments on the MinIO host, link avatars and the desktop app's native part keep working without configuration.

A policy you set for the app host on your own proxy is enforced in addition to this one: a request has to pass both. If it sets `connect-src` or `default-src`, it must let the desktop app reach its native part: add `ipc:` (Linux) and `http://ipc.localhost` (Windows) to `connect-src`, for example `connect-src 'self' ipc: http://ipc.localhost`. Without `connect-src`, `default-src` applies. Without these sources the app falls back to a slow path: it keeps working, but video in its tiles stays at 320×180 with 15 frames per second, and the tile reports "Geringe Bildqualität" (low picture quality) with the Content-Security-Policy as the reason. Browsers and the installed PWA are not affected. To drop the stack's policy, remove the header on your proxy: Caddy `header -Content-Security-Policy` in the app's site block, Nginx `proxy_hide_header Content-Security-Policy;`.

**Without the Compose Caddy:** the overlay publishes web/api/media (and MinIO) on loopback. Template: `deploy/compose/Caddyfile.homelab` (two active site blocks: app + MinIO).

Important: the Compose `.env` does **not** automatically export variables to an external Caddy service or to a Caddy container in another stack. Setup:

1. Copy/import both site blocks from `Caddyfile.homelab` into the Caddy configuration actually in use.
2. Set the domain and upstream on the Caddy service/container **or** write them into the Caddyfile. Keep host and URL apart:
   - `GELABBER_MINIO_DOMAIN=minio.example.com` (Caddy site / host)
   - `MINIO_PUBLIC_ENDPOINT=https://minio.example.com` (API presign URL, scheme included)
3. With a changed `MINIO_API_PORT`, adjust the upstream; in the Docker network `gelabber` the upstream is `minio:9000` (no host port needed). App upstreams likewise: `web:80`, `api:8080`, `media:8081`.
4. MinIO block: `header_up Host {hostport}` (keep the port, for example `:8443`), URI unchanged; presigns sign host and path.
5. Behind a CDN or tunnel, add the global `servers { trusted_proxies … }` snippet from the top of `Caddyfile.homelab` to your Caddy, with the CDN's ranges; otherwise every client counts as the CDN (see "Client addresses"). Keep `GELABBER_API_BIND` on loopback: the API believes `X-Forwarded-For` from whoever reaches its port.
6. Validate the configuration (`caddy validate` / `caddy adapt`) and reload Caddy.
7. With `.home.arpa`: make the browser clients trust the local CA ([Local HTTPS](https://caddyserver.com/docs/automatic-https#local-https)).

Environment variables in Caddy: [Caddyfile environment variables](https://caddyserver.com/docs/caddyfile/concepts#environment-variables).

```bash
cp deploy/compose/.env.homelab.example deploy/compose/.env
# Set the app and MinIO domains, IPs and secrets; both names must be in DNS.
# Give Caddy both site blocks + the same domain/upstream values separately.
cd deploy/compose
docker compose up -d
```

## TURN credentials

Two modes. They must be the same on the API and on coturn.

Compose offers TURN over UDP and TCP. TCP is the fallback for clients whose network blocks UDP to the server; coturn still relays from there to the SFU over UDP. With your own `TURN_URLS` entry, include both variants, for example `stun:example.com:3478,turn:example.com:3478?transport=udp,turn:example.com:3478?transport=tcp`. An image update does not extend existing `.env` overrides. TCP 3478 must be reachable next to the documented UDP ports; the bundled coturn publishes both.

**Static (default).** Leave `TURN_AUTH_SECRET` empty. The ticket sends `TURN_USERNAME` / `TURN_PASSWORD`. The bundled coturn checks exactly these long-term credentials (`--lt-cred-mech`). An external TURN that already knows the user/password stays reachable this way, even when the homelab overlay turns the bundled coturn off.

**REST.** Set the same private secret on the API and on coturn (`TURN_AUTH_SECRET`, coturn `--use-auth-secret` / `--static-auth-secret`). Every ticket then gets time-limited HMAC credentials (`expiry:user`) instead of the static password. `TURN_CRED_TTL_SECS` (default 21600) is their lifetime. The bundled coturn switches to REST on its own.

`gelabberturnsecret` is not a secret. The old Compose default wrote it into every stack; the API and the bundled coturn no longer start with it. If it is in your `.env`, delete the line (static) or set your own secret on both sides.

### Migration

1. Only `TURN_USERNAME` / `TURN_PASSWORD` so far (external TURN, homelab without a rebuilt coturn): do not set `TURN_AUTH_SECRET`. Relays keep the existing credentials.
2. Bundled coturn without your own `TURN_AUTH_SECRET` line: add nothing. The API and coturn move together from the public REST default to static user/password credentials.
3. REST wanted: generate a new, non-public secret, write it identically into the API environment and into coturn (`static-auth-secret`), restart both. Only then issue tickets with HMAC credentials. An external TURN must get the same secret before the variable is set.

### After changing TURN values

The API and coturn read `TURN_*` from `.env` only when their containers are created. Watchtower swaps images and keeps each container's old environment, and `up … api` recreates only the services it names. After any change to `TURN_AUTH_SECRET`, `TURN_USERNAME` or `TURN_PASSWORD` (also when a release removes a default, as v0.6 did for `TURN_PASSWORD`), recreate both:

```bash
cd deploy/compose
# Homelab overlay without COMPOSE_PROFILES=bundled-coturn: leave out `coturn`
# (naming it starts the bundled one) and update your own TURN server instead.
docker compose up -d --no-deps --force-recreate coturn api
./turn-check.sh
```

For a digest rollout use the same `--env-file` / `-f` arguments as the rollout. If you started the bundled coturn by mistake on the homelab overlay, remove it with `docker compose rm -sf coturn`.

`turn-check.sh` compares the values the running API and coturn were started with and names the keys that differ, without printing them. A mismatch looks like a working call without sound: the client connects, but coturn refuses the relay. With an external TURN (homelab overlay without `bundled-coturn`) the script only reminds you to check that server yourself.

## TURN port in use

`Bind for 0.0.0.0:3478 failed`: usually a coturn is already running.

- The overlay (way 2) does not start Gelabber's coturn. Point `TURN_PUBLIC_HOST` / `TURN_PORT` / user / password at the existing server. From a container, `127.0.0.1` is wrong (use the LAN IP or `host.docker.internal`).
- A second coturn: `COMPOSE_PROFILES=bundled-coturn` plus a free `TURN_PORT` (for example 3479) and relays (`TURN_RELAY_MIN` / `TURN_RELAY_MAX`). `TURN_URLS` goes on the API ticket; the SFU does not read it.
- Without the overlay: only `TURN_PORT=3479` in `.env` (and free relays).

## Media signaling

The SFU's control WebSocket (`/media/ws`) closes a socket that has not joined a voice channel within `MEDIA_JOIN_DEADLINE_MS` (default 10000 ms), counted from the upgrade and again from a leave. Pings and invalid frames do not extend it. The web client sends its join right after the socket opens, so Compose leaves the key at its default; to change it, add it to the `media` service's `environment` in a Compose override.

Media expects to run behind the reverse proxy. Its HTTP server has no read timeout for the request and header phase, so a client that connects to `media:8081` directly and never finishes its request keeps the connection open. Caddy connects upstream only once it has the full request headers, so this matters only when port 8081 is reachable from outside: keep `GELABBER_MEDIA_BIND` (homelab overlay) on loopback.

## Behind TLS

- `API_COOKIE_SECURE=true`. The API then accepts the WebSocket (`/ws`) only from `https://` pages; `http://` stays allowed for `localhost` only.
- `GELABBER_MINIO_DOMAIN` (Caddy host, for example `minio.example.com`) and `MINIO_PUBLIC_ENDPOINT` (full URL, for example `https://minio.example.com`) must match the active MinIO site in the **real** Caddy config (the Compose `.env` alone is not enough for an external Caddy); no path prefix, presigns sign host and path. `MINIO_API_CORS_ALLOW_ORIGIN` = the Gelabber origin
- `TURN_PUBLIC_HOST`, `TURN_EXTERNAL_IP`, `MEDIA_ADVERTISED_IP` = the address clients reach

### MinIO user

Since #196 the API signs in to MinIO as a bucket-scoped user, not as root. `minio-init` (image `ghcr.io/till-und-scholler-ai-slop/gelabber/mc`, the pinned MinIO client built from source like the server; `GELABBER_MC_IMAGE` overrides it) runs before the API on every `docker compose up`. It creates the bucket if missing, writes the policy `gelabber-app-<bucket>`, sets the user's password from `.env` and attaches the policy. It never touches objects, so it is safe on an existing volume and on every rerun.

**Upgrading an existing install** (data stays where it is, same bucket, same volume):

1. Nothing breaks if you do nothing: a new API image under the old `compose.yaml` (for example Watchtower pulling `latest`) still accepts `MINIO_ROOT_USER` / `MINIO_ROOT_PASSWORD`, but logs a warning at startup that it runs as root.
2. Update the checkout to the release tag and add a new password to `.env`:
   ```bash
   cd deploy/compose
   grep -q '^MINIO_APP_PASSWORD=' .env || echo "MINIO_APP_PASSWORD=$(openssl rand -hex 24)" >> .env
   ```
   Without it, `docker compose` stops with `set MINIO_APP_PASSWORD in .env`. `MINIO_APP_USER` defaults to `gelabber-api` and must differ from `MINIO_ROOT_USER`.
3. `docker compose up -d`. Compose starts `minio-init` after MinIO is healthy and recreates the API with the new user once `minio-init` has finished. `docker compose logs minio-init` ends with `MinIO user gelabber-api may use bucket gelabber only.`; the API log no longer shows the root warning.
4. Check in the app that an existing attachment still loads and a new upload works.

If `minio-init` fails (for example `MINIO_APP_PASSWORD` shorter than 8 characters), Compose has already removed the old API container and does not start the new one: Gelabber is down until you fix `.env` and run `docker compose up -d` again. `docker compose logs minio-init` names the cause. Changing `MINIO_APP_PASSWORD` later: edit `.env`, `docker compose up -d` (`minio-init` sets the new password, the API is recreated with it). Renaming `MINIO_APP_USER` leaves the old user in MinIO; remove it with `mc admin user remove` if you want. Links the API signed as root before the switch stay valid until they expire, because root still exists. A changed `MINIO_APP_PASSWORD` ends links signed with the old one at once: open pages show missing images until a reload, and uploads in flight fail once.

Rollback to an API before v0.7 needs the old `compose.yaml` too (it reads only `MINIO_ROOT_*`); see "Rollout and rollback". The MinIO user can stay, it is unused then.

### MinIO on 443

Browsers upload and download attachments directly from MinIO through presigned URLs, so `MINIO_PUBLIC_ENDPOINT` is public by design. Put it on its own subdomain behind your TLS Caddy, not on a published port such as `https://example.com:9000`: networks that only allow 80/443 then load no images, and the S3 API (plus the console on 9001, if published) faces the internet without the proxy in front. The move fixes reachability and removes the directly published 9000/9001 (console included), but it does not shrink the S3 API's attack surface: the Caddy MinIO site forwards every path and method, so `https://minio.example.com` still serves the whole S3/admin API. Since #196 the API signs with a bucket-scoped user instead of root, so presigned links and a leaked API environment reach only the Gelabber bucket; the root password still guards the admin API, keep it private. The API logs a warning at startup when an `https://` endpoint carries a port other than 443.

1. DNS for `minio.example.com`, and the MinIO site from `Caddyfile.homelab` in your Caddy with `minio.example.com` as its address (the default is `minio.home.arpa`; `header_up Host {hostport}`, URI unchanged).
2. `.env`: `MINIO_PUBLIC_ENDPOINT=https://minio.example.com`, `MINIO_API_CORS_ALLOW_ORIGIN=https://example.com`. Leave `MINIO_API_BIND` and `MINIO_CONSOLE_BIND` as they are for now: open browsers cache the attachment redirect for the rest of its signing window and keep following the old `:9000` link for up to 30 minutes, so the old port must stay reachable that long.
3. `docker compose up -d minio api`. Check from outside that `curl -I https://minio.example.com/minio/health/live` answers 200, and that an attachment link now redirects to `https://minio.example.com/…`: with `curl -I` and your session cookie, or in a private window.
4. About 30 minutes later, set `MINIO_API_BIND=127.0.0.1` and `MINIO_CONSOLE_BIND=127.0.0.1` in `.env`, run `docker compose up -d minio`, and close 9000/9001 in the firewall. Check from outside that `curl -I https://example.com:9000/minio/health/live` fails. In the app, upload a new image and check that it loads (its redirect cannot come from a cache), then hard-reload (Ctrl+Shift+R) and check that an existing image still loads.

If you cannot wait, do step 4 right after step 3: browsers that were already open may then miss some images for up to 30 minutes, until their cached redirects run out or the page is hard-reloaded.

A path prefix (`https://example.com/s3/…`) does not work: presigns sign host and path.

## Attachments

A message carries up to 10 files, at most 25 MiB each and 100 MiB together. Uploads count against a per-user daily quota, `API_UPLOAD_QUOTA_BYTES_PER_DAY` (default 1 GiB, `0` turns it off):

- The size is reserved when the upload is signed.
- An upload that never joins a message waits up to one hour for it (ten files go up one after another on slow lines), plus 15 minutes for an unfinished transfer. Only then is it removed: bytes that never arrived go back to the quota, bytes that did stay counted.
- So a user who abandons a large set can see "quota exceeded" for up to 75 minutes afterwards.
- Avatar and banner uploads count against the same quota. Their bytes are counted when the image is uploaded and stay counted, even if storing it fails. A user who has used up the day's quota cannot upload a new avatar or banner until the next UTC day; removing either, or setting an avatar link, still works.

## Link previews

Links in messages get a preview card (title, description, image; YouTube with a thumbnail and a player that loads only on click). The API fetches the pages itself, in the background after sending: up to three links per message, only public addresses on ports 80 and 443, every redirect checked again, 5 s and 1 MB per page, 5 MB per image. Private, loopback, link-local, CGNAT and multicast addresses, single-label and local names (`postgres`, `minio`, `*.internal`, …) and the hosts of the database, Redis, MinIO and TURN settings are refused; add more with `API_LINK_PREVIEW_DENY_HOSTS`. Images are copied into MinIO under `previews/` and served by the API, so browsers never contact the linked site. At most 8 s per message; a site that times out is skipped for 10 minutes. Results are cached for a day, and a message keeps the card it got even when the page changes later; unused previews are dropped after a week. Links to the instance itself get no preview.

The linked sites see the server's IP address. `API_LINK_PREVIEWS=false` turns previews off; links stay clickable. Authors (and moderators) can remove the previews of a message.

## GIFs

GIF search and trending in chat and direct messages come from [KLIPY](https://klipy.com) and are off until `KLIPY_API_KEY` is set in `.env`. Without it `GET /api/gifs` reports `enabled: false`, the composer shows no GIF button and the search, trending and media routes answer 404.

The API makes every KLIPY call itself and relays the GIF images from KLIPY's media hosts (`static*.klipy.com`) under `/api/gifs/media`; browsers and the desktop app talk only to Gelabber and never see the key. KLIPY sees each user only as a hash of the account ID and the key, not as a name or address. A sent GIF is an ordinary chat message that stores KLIPY's slug, size and media URLs; it does not touch voice signaling or the SFU.

KLIPY's terms require their written approval for server-side requests and for proxying or caching their media (developers@klipy.com). Get it before you set the key. Testing keys are limited to 100 requests per hour, shared by every user of the instance.

- `KLIPY_CONTENT_FILTER`: `off`, `low`, `medium` (default) or `high`.
- `API_RATE_GIF_PER_MIN`: search, trending and sent GIFs (each a KLIPY API request) per user per minute (default 30, `0` turns the limit off). It is per user; it does not keep the whole instance under a testing key's 100 per hour.
- `API_RATE_GIF_MEDIA_PER_MIN`: image loads through the relay per user per minute (default 240, `0` turns the limit off). They go to KLIPY's media hosts, not its API. At most 128 relay streams run at once across all users; past that the relay answers 502.

When KLIPY itself answers 429, the key's quota is used up for every user. The API logs it, counts it as `gelabber_rate_limited_total{bucket="klipy"}` and answers the client 502 `upstream_unavailable`, not as the user's own limit.

After changing these values, recreate the API: `docker compose up -d --no-deps --force-recreate api`.

Migration 0012 adds the nullable `messages.gif` column. As with every migration, an older API binary does not start against the migrated database (SQLx ledger); roll back with the backed-up snapshot pair as described below.


## Push notifications

Direct messages and mentions can reach a phone or browser whose Gelabber tab or PWA is closed (Web Push). It is off until `VAPID_PRIVATE_KEY` and `VAPID_SUBJECT` are set in `.env`:

```sh
docker compose run --rm api gelabber-api vapid-keys
```

Copy the printed `VAPID_PRIVATE_KEY=…` line into `.env`, set `VAPID_SUBJECT=mailto:you@example.org` (a contact the push services can reach), then `docker compose up -d api`. Keep the key: a new one silently ends every existing subscription, and people only get pushes again after they open Gelabber once more.

What is pushed: every direct message, and in servers only mentions (`@name`, `@everyone`, `@here`). Nothing is pushed for a conversation someone muted or set to "nothing", nor while any page of theirs is in front on some device. A push holds the sender, the channel and up to 180 characters of the message, encrypted for that browser (RFC 8291); the push service (Google, Mozilla, Microsoft or Apple) sees only who to wake and when. The API posts only to those services' hosts; `PUSH_ALLOWED_HOSTS` changes the list. Signing out ends the pushes for that browser.

Limits: `@everyone` and `@here` need the server permission "@everyone und @here" (owners always have it) and count at most 3 per author and server in 10 minutes. `@everyone` pushes at most 5 times per server in 10 minutes, and in a server with more than 500 members it pushes only to the people named in the message. At most 16 messages are being pushed at once; a burst beyond that is dropped and counted as `gelabber_rate_limited_total{bucket="push_dropped"}`. A page counts as in front while it is visible and someone used it in the last 5 minutes. Subscriptions that keep failing are removed once they have not taken a push for 30 days.

On iPhone and iPad (iOS 16.4 or later), push works only in Gelabber added to the home screen, not in a Safari tab.

## Account mail

With outgoing mail, new accounts confirm their address and people who forgot their password get a reset link. It is off until `SMTP_HOST` is set in `.env`; the API then sends through that relay (your mail provider, a transactional mail service, or a Postfix on the same network). Gelabber runs no mail server of its own.

```sh
SMTP_HOST=smtp.example.org
SMTP_USERNAME=gelabber@example.org
SMTP_PASSWORD=…
MAIL_FROM=Gelabber <gelabber@example.org>
APP_PUBLIC_URL=https://chat.example.org
```

- `SMTP_SECURITY`: `starttls` (default, port 587, the upgrade must succeed), `tls` (port 465) or `none` (port 25, only for a relay on a private network; refused together with `SMTP_PASSWORD`). `SMTP_PORT` overrides the port.
- `MAIL_FROM`: the sender. Most providers only accept addresses they host; set SPF/DKIM for its domain or the mails land in spam.
- `APP_PUBLIC_URL`: the address people open Gelabber at. The links in the mails point there, never at the host a request named. It must be `https://` while `API_COOKIE_SECURE=true`.

Then `docker compose up -d --no-deps --force-recreate api`. A wrong value stops the API at start with the variable's name; an unreachable relay does not, failed mails are logged (`account mail failed`, with the user id, never the address).

What changes with mail on:

- Registration answers the same whether or not the address already has an account ("check your inbox"), so it no longer tells strangers who is registered. A new address gets a confirmation link (valid 24 hours); a confirmed one gets a note that the account exists, with links to sign in and to reset the password, and the account does not change. A registration that nobody confirmed yet is replaced by the newer one (name, password, a fresh link), so a stranger who registers someone's address first never holds the password of the account its owner later confirms.
- An unconfirmed account cannot sign in. The right password answers "confirm your address first" and sends a new link. The link page asks for the account's password; with it the address is confirmed and that browser signed in. The link alone is not enough, so someone who registered a stranger's address cannot share the account the stranger confirms, and mail scanners that open links do not use it up. A wrong password counts like a failed sign-in and leaves the link usable.
- "Passwort vergessen?" on the sign-in page mails a reset link (valid one hour, once) if the address has an account; the answer is the same either way. Setting the new password confirms the address, ends every session of the account on every device and signs this browser in. Changing the password in the settings makes open reset links invalid.
- At most 5 mails per address and hour; further requests are answered as usual and nothing is sent (`gelabber_rate_limited_total{bucket="mail"}`). The links carry their token after `#`, so it does not reach proxy logs, and only its SHA-256 is stored.

Accounts that existed before migration 0021 count as confirmed. Accounts registered later while mail was off are unconfirmed: once mail is turned on, their owners confirm once at their next sign-in. Without mail, registration signs in at once and answers `409` for a known address, as before.

Migration 0021 adds `users.email_verified_at` and the `account_tokens` table. As with every migration, an older API binary does not start against the migrated database; roll back with the backed-up snapshot pair as described below.

## Database connections

The API keeps up to `API_DB_MAX_CONNECTIONS` (default 20) Postgres connections; every open WebSocket checks its session once a second. The stack runs one API process, so keep the value well below Postgres' `max_connections` (100 in the bundled Postgres). After changing it, recreate the API: `docker compose up -d --no-deps --force-recreate api`.

## Backup

PostgreSQL metadata and MinIO attachments form **one snapshot pair**. Before
the backup, stop all app writers, including API background jobs and still
valid direct MinIO uploads. Drain the API first, then stop MinIO; only then
back up both data sets. A `tar` of a running PostgreSQL data volume is not a
consistent physical backup. Here we use `pg_dump` with the complete SQLx
migration ledger instead. Redis is volatile and not part of a data restore.

Example for the Compose stack with a maintenance window (the MinIO volume
actually mounted and the running image are determined before the stop):

```bash
cd deploy/compose
sh ../backup.sh
```

Encrypt backups that leave the server: with
`GELABBER_BACKUP_AGE_RECIPIENT=age1…` (an [age](https://age-encryption.org)
public key) the dump and the MinIO archive are encrypted while they are
written, inside the maintenance window, and stored only as `*.age`; they never
reach the disk in plain text. A recipient that age does not accept ends the
script before anything is stopped. `SHA256SUMS` names the files as stored, so
it can be checked without the key: before a restore, run
`sha256sum -c SHA256SUMS`, then decrypt with `age -d -i key.txt -o
database.pgdump database.pgdump.age` (likewise `minio-data.tgz`). Backups made
by 0.7.0 or earlier list the plain names; check those after decrypting.

On an error the writers stay stopped until the incomplete backup has been
examined. A backup directory without `SHA256SUMS`, or with `*.part` files, is
incomplete. Archive the released `image-set.json`/`image-set.env`, the active
Compose configuration and the backup time with the pair; keep secrets private.
A local image ID alone is not a portable replacement for the archived registry
digest.

Test a restore on **fresh targets** first: `sha256sum -c SHA256SUMS`, then fill
a new empty PostgreSQL database and a new empty MinIO volume with
`pg_restore --exit-on-error --no-owner` and the fully extracted MinIO archive
respectively. Start the object store with the same MinIO pin and configure the
matching API/image version for this pair. Keep the source data until byte
hashes, file type, message metadata, membership, DM scope and refused
downloads/uploads have been checked. Use new presigns, not expired download
URLs from the backup.

Going back to an old snapshot with a binary/image deliberately loses all
changes after its time. Starting an old binary against the new schema is a
separate gate: even additive SQL changes can fail on the SQLx migration
ledger. That is why an image change does not replace the tested restore of the
matching old DB/object store pair.

The full local drill is described in [Upgrade/Storage/Restore](../docs/upgrade-storage-restore.md).
It uses only its own throwaway containers and runs a real rollback to an old
API binary; the older `tools/check-chat-migration-restore.py` stays available
as a quick schema/data drill without object store or binary acceptance.

## Metrics

No debug UI in `web`. Grafana does not start with `docker compose up`:

```bash
cd deploy/compose
docker compose -f compose.yaml -f compose.observability.yaml up -d
# Homelab: a third -f compose.homelab.yaml
```

Grafana 13.2.1: `http://127.0.0.1:3000/d/gelabber/gelabber` (admin / `gelabber`, anonymous viewer). File: `deploy/compose/grafana/dashboards/gelabber.json`. Prometheus v3.14.0: `http://127.0.0.1:9090`, scrapes `api:8080/metrics` and `media:8081/metrics`. Not behind Caddy. Process tracing is JSON on stdout (`RUST_LOG`), no Jaeger.

## Images and release gate

The API and media Docker builds keep compiled external Rust dependencies in a layer cache stage of their own. The recipe takes the manifests, the lockfile and the toolchain, but contains only placeholder sources. Only the versions of the three local workspace packages are set to `0.0.0` in this temporary recipe; external versions, checksums and features stay as they are. Source changes and pure app version bumps therefore do not invalidate the dependency layer. Before the actual build, the placeholder artifacts are removed with a per-package `cargo clean`, and all original manifests, the original lockfile and the real sources are taken over. The finished build still uses `--locked` and the actual release version. API source changes do not invalidate the media build and vice versa; changes to `shared` affect both.

The existing separate GitHub Actions layer caches for API and media stay. Cold builds have to compile the dependencies once; warm builds can reuse them. Cache recipe checks run with the deployment configuration tests; actual run times and container readiness must also be checked with Docker.

The slow local cache acceptance starts with `python3 docker/check-rust-cache.py --baseline-ref <commit-before-cache-change> --output /tmp/gelabber-rust-cache`. It checks source, shared, version and feature changes in buildx caches of their own and compares three different source rebuilds with the previous Dockerfiles. JSON and full logs contain the cache hits and the measured run times of the build stage, without image export; at least 20 percent improvement is checked. `--keep-images` then exports images from the unchanged source state for a separate readiness check. This runner belongs to local acceptance and does not run in CI.

The Compose default is the published version `v0.7.1`. Existing release tags are not overwritten. MinIO independently uses the existing CE pin `RELEASE.2025-10-15T17-29-55Z`. Org packages can be private: `docker login ghcr.io` or build locally. All stack pins stay. The source build sets `CARGO_HTTP_CAINFO`; with TLS inspection, `docker/rust-build-ca.sh` appends the presented chain.

CI runs for **every main commit**, so that deploy/workflow changes have a definite CI SHA too. PR path filters cover `shared/**`, all workspace members, Docker contexts, lockfiles and workflows. The image workflow builds PRs without pushing. On main it starts only after a successful `CI` push run of the same SHA; failed/cancelled CI starts no publish job.

First, app tags `sha-<full SHA>-<run ID>-<attempt>` are created; OCI labels contain revision, version, commit time and source URL. The extra run fields prevent a rebuild of the same SHA from overwriting an earlier build. The promotion checks CI again and checks **all three** version digests before the first write. An existing version digest must be exactly identical. An existing Git release tag with another SHA, or main having moved on in the meantime, leads to a plain candidate set without version promotion. The build/promotion run writes no `latest`. Only the successful `Release` workflow sets the `latest` alias for API, web, media and MinIO to the digests of the published stable release. Before that it checks the whole set, the Git tag, CI and the publish run; older releases and candidates must not reset the aliases. MinIO keeps its separate pin; its `latest` points to the MinIO digest from the release set.

The `image-set` artifact contains:

- `image-set.json`: revision, version, CI/publish run, promotion status, API/web/media digests and the separate MinIO pin/digest.
- `image-set.env`: four `GELABBER_*_IMAGE=ghcr.io/…@sha256:…` references for Compose.

Candidate sets are **not releases**. A finished hotfix gets a new workspace/Compose version only after coordinated acceptance. The same version belongs in `desktop/Cargo.toml` and `desktop/Cargo.lock`: the desktop apps (Windows installer, Linux tarball, pacman package) carry the version of this workspace of their own, `Release` stops on a different one before tag and release are created, and the desktop workflow attaches the apps only to a release with exactly their version. The coordinator then starts the `Release` workflow **on main** with the matching `tag` and a successful `image_run_id`. It checks manifest, CI, the current main state and version digests again, keeps existing tags/releases and attaches the set to the new GitHub release. There is no automatic release on every push and no release of feature/major intermediate states. No trigger by arbitrary `v*` tag pushes.

If an alias update was interrupted or was missing for an older release: start `Release` on **main** with its `tag` and `image_run_id` and turn on **refresh_latest_only**. That repairs only the aliases of the stable GitHub release currently published as latest, without builds, new releases or changes to fixed tags. For v0.2.5: `tag=v0.2.5`, `image_run_id=36549037951`. All four aliases are written one by one; on an error the workflow stays red and can be started again.

## Rollout and rollback with exact digests

After acceptance, download the set from the released release and archive it permanently with the deployment. For the first migration, record the previous set **actually running**: capture `docker compose images` / container inspect and registry digests; do not assume that an old mutable tag matches the running container. Save this old set as `previous.env` (all four image variables). Credentials stay in `.env` only, not in the manifest.

```bash
cd deploy/compose
# Put the released image-set.env here as next.env.
# .env holds the secrets as before, and COMPOSE_FILE for homelab if used.
docker compose --env-file .env --env-file next.env config -q
docker compose --env-file .env --env-file next.env config --images
# Backup as in the Backup section; keep the old manifest/env files.
docker compose --env-file .env --env-file next.env pull --policy always api web media minio minio-init
docker compose --env-file .env --env-file next.env up -d --no-deps --no-build --pull never --force-recreate minio minio-init api web media
# Bundled coturn only (skip with the homelab overlay unless COMPOSE_PROFILES=bundled-coturn):
docker compose --env-file .env --env-file next.env up -d --no-deps --force-recreate coturn
./turn-check.sh
# Then: /ready, /media/ready, login, upload and a real media/relay acceptance.
```

coturn is not part of the image set, but it carries the TURN credentials from `.env`. Without this step it keeps the values from its last start while the API hands out the new ones (see "After changing TURN values").

`--no-build` prevents an unnoticed local replacement build; the explicit pull solves the earlier `pull_policy: missing` problem. Remove `GELABBER_*_IMAGE` variables already exported in the shell environment first, since they override env files. Homelab keeps its `COMPOSE_FILE`; alternatively use the same `-f` overlays with **all** commands. Images are pulled ahead as a set; container switches are not atomic and need a maintenance window.

**The deploy files belong to the set.** The commands above swap only images; `Caddyfile` and `compose.yaml` come from the checkout. If a release changes them, bring the checkout to its tag before these commands and, after the image switch, recreate the proxy too, **only where the bundled Caddy runs** (default Compose without the homelab overlay):

```bash
# Do not run with compose.homelab.yaml (see below).
docker compose --env-file .env --env-file next.env up -d --no-deps --force-recreate proxy
```

A `caddy reload` is not enough for this. The proxy mounts `./Caddyfile` as a single file, and `git` replaces the file when updating: the running container keeps seeing the old one, and a reload loads the old text again. Only the restarted container (`docker compose restart proxy` is enough when only the Caddyfile changed) reads the new one. **v0.6 is such a release:** the Caddyfile hands `Host` on with its port (`{hostport}`, see "Existing Caddy") and compresses responses. Without this step both stay off and arrive unannounced with the container's next restart. **So is v0.7.1:** its Caddyfile and `compose.yaml` hand the API the client address from behind a proxy (see "Client addresses").

**Do not run this step with the homelab overlay (`compose.homelab.yaml`, your own Caddy in front).** The overlay only puts the bundled `proxy` into the `bundled-caddy` profile; a service named explicitly starts anyway. `up … proxy` would therefore start the bundled Caddy on `${GELABBER_HTTP_BIND:-0.0.0.0}:${GELABBER_HTTP_PORT:-80}`: next to your own Caddy, which usually holds port 80 already (the start fails), or open on the network without TLS. Homelab operators instead carry the change from `deploy/compose/Caddyfile.homelab` (v0.6: `header_up Host {hostport}`) into their own Caddy's configuration and reload it there (v0.7.1: `header_up X-Forwarded-For {client_ip}` on the API upstreams).

**v0.4 → v0.3.1 does not work through an image change:** migrations
0010/0011 change the SQLx migration ledger. Before starting v0.3.1, the
PostgreSQL/MinIO snapshot pair backed up before the upgrade must be restored
to tested targets. Changes since that backup are lost.

Only with proven schema/ledger compatibility, use the archived previous
complete API/web/media set (no single old media image):

```bash
docker compose --env-file .env --env-file previous.env config -q
docker compose --env-file .env --env-file previous.env pull --policy always api web media minio minio-init
docker compose --env-file .env --env-file previous.env up -d --no-deps --no-build --pull never --force-recreate minio minio-init api web media
```

Here too the Caddyfile belongs to the set. The v0.6 Caddyfile (`{hostport}`) fits an API from 0.6 on: an older API compares `Origin` and `Host` literally and refuses every WebSocket handshake with 403 as soon as a proxy in front writes out the default port (`Host: example.com:443`). With the old Caddyfile the same configuration worked. For a rollback to 0.5.x, therefore also reset the checkout to the old tag and recreate the proxy as above, with `previous.env`; with the homelab overlay, restore the old line in your own Caddy instead. The same holds for `compose.yaml` from v0.7 on: it hands the API only the bucket-scoped MinIO user (#196), which an API before v0.7 does not read, so for a rollback below v0.7 reset the checkout to the old tag before the commands above (the old one has no `minio-init` service; drop it from the commands).

No `down -v`, do not delete or rename volumes: Postgres/MinIO data stays on the same volumes. An image rollback does **not** replace a database restore; before releases, migrations must be checked for backward compatibility and backups made. After incompatible migrations the separately tested restore is required. Readiness alone is not a media/storage acceptance.

The pipeline prevents overwrites within these workflows through a shared promotion/release lock and digest checks. GHCR thereby offers no transaction-wide immutability against external maintainer pushes. With an aborted promotion, single new version tags may already exist; the run then delivers no released set, and `Release` stays locked. Rebuilds with other digests are locked as well, instead of repairing/overwriting tags. CI/artifact retention is 90 days; archive released sets permanently as well.

Local check for Auftrag09: [evidence and limits](stability-09-evidence.md). Build identity is available through OCI/manifest; health responses and product source code were not changed. Compose/browser smoke tests run locally: after starting the local stack, in `web` with `npm run test:browser-smoke` or `npm run test:e2e-app`. The Compose smoke job was removed from GitHub Actions; regular CI checks Rust, frontend and deployment configuration.
