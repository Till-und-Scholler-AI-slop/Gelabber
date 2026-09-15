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

`header_up Host {host}` ist Pflicht. `/ws` vergleicht Browser-`Origin` mit `Host`; ohne Header wird `Host` zum Upstream und der Handshake ist 403.

**Ohne Compose-Caddy:** Overlay published web/api/media auf Loopback. Vorlage: `deploy/compose/Caddyfile.homelab`.

```bash
cp deploy/compose/.env.homelab.example deploy/compose/.env
# Domain, IPs, Secrets, MINIO_PUBLIC_ENDPOINT setzen
cd deploy/compose
docker compose up -d
```

Caddy in Docker: Stack-Netz `gelabber`, Upstreams `web:80`, `api:8080`, `media:8081` (`minio:9000` braucht keinen Host-Port).

## TURN-Port belegt

`Bind for 0.0.0.0:3478 failed` — meist schon ein coturn.

- Overlay (Weg 2) startet Gelabbers coturn nicht. `TURN_PUBLIC_HOST` / `TURN_PORT` / User / Pass auf den bestehenden Server. Aus einem Container ist `127.0.0.1` falsch (LAN-IP oder `host.docker.internal`).
- Zweiter coturn: `COMPOSE_PROFILES=bundled-coturn` plus freien `TURN_PORT` (z. B. 3479) und Relays (`TURN_RELAY_MIN` / `TURN_RELAY_MAX`), `MEDIA_TURN_URLS=stun:coturn:3478,turn:coturn:3478`.
- Ohne Overlay: in `.env` nur `TURN_PORT=3479` (und freie Relays).

## Hinter TLS

- `API_COOKIE_SECURE=true`
- `MINIO_PUBLIC_ENDPOINT` = URL, die der Browser wirklich öffnet (eigene Subdomain, kein Path-Prefix — Presigns signieren den Host), plus `MINIO_API_CORS_ALLOW_ORIGIN` auf die Gelabber-Origin
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

`docker compose up` zieht `v0.1.3` von GHCR. Org-Pakete sind oft privat — dann Source-Build, oder `docker login ghcr.io`. MinIO bleibt der Pin in `deploy/compose/minio` (kein `FROM minio/minio`). Source-Build setzt `CARGO_HTTP_CAINFO`; bei TLS-Inspection (Docker Desktop) hängt `docker/rust-build-ca.sh` die präsentierte Kette an.
