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

## TURN-Port belegt

`Bind for 0.0.0.0:3478 failed` — meist schon ein coturn.

- Overlay (Weg 2) startet Gelabbers coturn nicht. `TURN_PUBLIC_HOST` / `TURN_PORT` / User / Pass auf den bestehenden Server. Aus einem Container ist `127.0.0.1` falsch (LAN-IP oder `host.docker.internal`).
- Zweiter coturn: `COMPOSE_PROFILES=bundled-coturn` plus freien `TURN_PORT` (z. B. 3479) und Relays (`TURN_RELAY_MIN` / `TURN_RELAY_MAX`), `MEDIA_TURN_URLS=stun:coturn:3478,turn:coturn:3478`.
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

```bash
cd deploy/compose
docker compose -f compose.yaml -f compose.observability.yaml up -d
```

Prometheus v3.14.0 und Grafana 13.2.1 auf Loopback (`http://127.0.0.1:3000`, admin / `gelabber`). Nicht hinter Caddy.

## Images

`docker compose up` zieht `v0.2.0` von GHCR. Org-Pakete sind oft privat — dann Source-Build, oder `docker login ghcr.io`. MinIO bleibt der Pin in `deploy/compose/minio` (kein `FROM minio/minio`). Source-Build setzt `CARGO_HTTP_CAINFO`; bei TLS-Inspection (Docker Desktop) hängt `docker/rust-build-ca.sh` die präsentierte Kette an.
