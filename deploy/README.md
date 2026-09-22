# Deploy

Ein Operator, ein Node. Stack: Caddy, Postgres, Redis, MinIO, coturn, api, web, media. Kein LiveKit.

```bash
cp deploy/compose/.env.example deploy/compose/.env
cd deploy/compose
docker compose up -d
```

App: `http://localhost` (Caddy :80). Daten und MinIO nur auf `127.0.0.1`. Secrets in `.env` ändern.

## Bestehendes Caddy

UDP (TURN + SFU-ICE 10000–10031) geht nicht durch Caddy — Host/Router-Ports bleiben offen.

**Kleiner Diff:** bundled Caddy auf Loopback, dein Caddy davor.

```bash
# deploy/compose/.env
GELABBER_HTTP_BIND=127.0.0.1
GELABBER_HTTP_PORT=8088
```

```
gelabber.example.com {
	reverse_proxy 127.0.0.1:8088 {
		header_up Host {host}
	}
}
```

`header_up Host {host}` ist für den App-vHost Pflicht. `/ws` vergleicht Browser-`Origin` mit `Host`. Ohne Override lässt Caddy bei **HTTP**-Upstreams den eingehenden `Host` standardmäßig durch; bei **HTTPS**-Upstreams setzt Caddy (ab v2.11) den Host auf den Upstream. Explizites Forwarding macht die Absicht klar und verhindert Signature-/Origin-Fehler. Siehe [reverse_proxy Headers](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy#headers).

**Ohne Compose-Caddy:** Overlay published web/api/media (und MinIO) auf Loopback. Vorlage: `deploy/compose/Caddyfile.homelab` (zwei aktive Site-Blöcke: App + MinIO).

Wichtig: Die Compose-`.env` exportiert **nicht** automatisch Variablen an einen externen Caddy-Dienst oder einen Caddy-Container in einem anderen Stack. Einrichtung:

1. Beide Site-Blöcke aus `Caddyfile.homelab` in die tatsächlich verwendete Caddy-Konfiguration kopieren/importieren.
2. Domain und Upstream am Caddy-Dienst/Container setzen **oder** fest in der Caddyfile eintragen. Host und URL unterscheiden:
   - `GELABBER_MINIO_DOMAIN=minio.example.com` (Caddy-Site / Host)
   - `MINIO_PUBLIC_ENDPOINT=https://minio.example.com` (API-Presign-URL, inkl. Schema)
3. Bei geändertem `MINIO_API_PORT` den Upstream anpassen; im Docker-Netz `gelabber` Upstream `minio:9000` (kein Host-Port nötig). App-Upstreams analog: `web:80`, `api:8080`, `media:8081`.
4. MinIO-Block: `header_up Host {hostport}` (Port erhalten, z. B. `:8443`), URI unverändert — Presigns signieren Host und Pfad.
5. Konfiguration validieren (`caddy validate` / `caddy adapt`) und Caddy neu laden.
6. Bei `.home.arpa`: lokale CA auf den Browser-Clients vertrauenswürdig machen ([Local HTTPS](https://caddyserver.com/docs/automatic-https#local-https)).

Umgebungsvariablen in Caddy: [Caddyfile environment variables](https://caddyserver.com/docs/caddyfile/concepts#environment-variables).

```bash
cp deploy/compose/.env.homelab.example deploy/compose/.env
# App- und MinIO-Domain, IPs und Secrets setzen; beide Namen müssen in DNS stehen.
# Caddy separat mit beiden Site-Blöcken + denselben Domain-/Upstream-Werten versorgen.
cd deploy/compose
docker compose up -d
```

## TURN-Zugangsdaten

Zwei Modi. Sie müssen auf API und coturn gleich sein.

**Statisch (Default).** `TURN_AUTH_SECRET` leer lassen. Das Ticket schickt `TURN_USERNAME` / `TURN_PASSWORD`. Der gebündelte coturn prüft genau diese Langzeit-Credentials (`--lt-cred-mech`). Ein externes TURN, das schon User/Passwort kennt, bleibt so erreichbar — auch wenn das Homelab-Overlay den gebündelten coturn abschaltet.

**REST.** Denselben privaten Secret auf der API und auf coturn setzen (`TURN_AUTH_SECRET`, coturn `--use-auth-secret` / `--static-auth-secret`). Jedes Ticket bekommt dann zeitlich begrenzte HMAC-Credentials (`expiry:user`) statt des statischen Passworts. `TURN_CRED_TTL_SECS` (Default 21600) ist die Gültigkeit. Der gebündelte coturn schaltet dabei von allein auf REST um.

`gelabberturnsecret` ist kein Secret. Der alte Compose-Default hat ihn in jeden Stack geschrieben; API und gebündelter coturn starten damit nicht mehr. Wer ihn in der `.env` stehen hat, löscht die Zeile (statisch) oder setzt ein eigenes Secret auf beiden Seiten.

### Migration

1. Bisher nur `TURN_USERNAME` / `TURN_PASSWORD` (externes TURN, Homelab ohne umgebauten coturn): `TURN_AUTH_SECRET` nicht setzen. Relays bleiben bei den bestehenden Credentials.
2. Gebündelter coturn ohne eigene `TURN_AUTH_SECRET`-Zeile: nichts eintragen. API und coturn wechseln zusammen von dem öffentlichen REST-Default auf statische User/Passwort-Credentials.
3. REST gewollt: ein neues, nicht öffentliches Secret erzeugen, identisch in die API-Umgebung und in coturn (`static-auth-secret`) schreiben, beide neu starten. Erst dann Tickets mit HMAC-Credentials ausstellen. Ein externes TURN muss denselben Secret bekommen, bevor die Variable gesetzt wird.

## TURN-Port belegt

`Bind for 0.0.0.0:3478 failed` — meist schon ein coturn.

- Overlay (Weg 2) startet Gelabbers coturn nicht. `TURN_PUBLIC_HOST` / `TURN_PORT` / User / Pass auf den bestehenden Server. Aus einem Container ist `127.0.0.1` falsch (LAN-IP oder `host.docker.internal`).
- Zweiter coturn: `COMPOSE_PROFILES=bundled-coturn` plus freien `TURN_PORT` (z. B. 3479) und Relays (`TURN_RELAY_MIN` / `TURN_RELAY_MAX`). `TURN_URLS` hängt am API-Ticket; der SFU liest sie nicht.
- Ohne Overlay: in `.env` nur `TURN_PORT=3479` (und freie Relays).

## Hinter TLS

- `API_COOKIE_SECURE=true`
- `GELABBER_MINIO_DOMAIN` (Caddy-Host, z. B. `minio.example.com`) und `MINIO_PUBLIC_ENDPOINT` (volle URL, z. B. `https://minio.example.com`) müssen zur aktiven MinIO-Site in der **echten** Caddy-Config passen (Compose-`.env` allein reicht für externes Caddy nicht); kein Path-Prefix — Presigns signieren Host und Pfad. `MINIO_API_CORS_ALLOW_ORIGIN` = Gelabber-Origin
- `TURN_PUBLIC_HOST`, `TURN_EXTERNAL_IP`, `MEDIA_ADVERTISED_IP` = Adresse, die Clients erreichen

## Backup

Volumes: `gelabber_postgres_data`, `gelabber_minio_data`. Redis speichert nichts.

```bash
cd deploy/compose
docker compose exec -T postgres pg_dump -U gelabber gelabber > gelabber-$(date -u +%Y%m%d).sql
docker run --rm -v gelabber_postgres_data:/data -v "$PWD":/backup alpine:3.24 \
  tar czf /backup/postgres-data.tgz -C /data .
docker run --rm -v gelabber_minio_data:/data -v "$PWD":/backup alpine:3.24 \
  tar czf /backup/minio-data.tgz -C /data .
```

Restore analog; Postgres vorher stoppen.

## Metriken

Kein Debug-UI in `web`. Grafana startet nicht mit `docker compose up`:

```bash
cd deploy/compose
docker compose -f compose.yaml -f compose.observability.yaml up -d
# Homelab: drittes -f compose.homelab.yaml
```

Grafana 13.2.1: `http://127.0.0.1:3000/d/gelabber/gelabber` (admin / `gelabber`, anonym Viewer). Datei: `deploy/compose/grafana/dashboards/gelabber.json`. Prometheus v3.14.0: `http://127.0.0.1:9090`, scrapet `api:8080/metrics` und `media:8081/metrics`. Nicht hinter Caddy. Prozess-Tracing ist JSON auf stdout (`RUST_LOG`), kein Jaeger.

## Images

`docker compose up` zieht `v0.2.1` von GHCR. Org-Pakete sind oft privat — dann Source-Build, oder `docker login ghcr.io`. MinIO bleibt der Pin in `deploy/compose/minio` (kein `FROM minio/minio`). Source-Build setzt `CARGO_HTTP_CAINFO`; bei TLS-Inspection (Docker Desktop) hängt `docker/rust-build-ca.sh` die präsentierte Kette an.
