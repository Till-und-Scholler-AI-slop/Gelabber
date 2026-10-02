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

Compose bietet TURN über UDP und TCP an. TCP ist der Ausweichpfad für Clients, deren Netz UDP zum Server blockiert; coturn leitet von dort weiterhin per UDP zum SFU weiter. Bei einem eigenen `TURN_URLS`-Eintrag beide Varianten aufnehmen, beispielsweise `stun:example.com:3478,turn:example.com:3478?transport=udp,turn:example.com:3478?transport=tcp`. Bestehende `.env`-Overrides werden durch ein Image-Update nicht ergänzt. TCP 3478 muss neben den dokumentierten UDP-Ports erreichbar sein; der gebündelte coturn veröffentlicht beide.

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

## Images und Release-Gate

Der Default zieht die App-Version `v0.3.0`; bestehende Release-Tags werden nicht überschrieben. MinIO verwendet unabhängig davon den bestehenden CE-Pin `RELEASE.2025-10-15T17-29-55Z`. Org-Pakete können privat sein: `docker login ghcr.io` oder lokal bauen. Alle Stack-Pins bleiben bestehen. Source-Build setzt `CARGO_HTTP_CAINFO`; bei TLS-Inspection hängt `docker/rust-build-ca.sh` die präsentierte Kette an.

CI läuft für **jeden main-Commit**, damit auch Deploy-/Workflow-Änderungen eine eindeutige CI-SHA besitzen. PR-Pfadfilter erfassen `shared/**`, alle Workspace-Mitglieder, Docker-Kontexte, Lockfiles und Workflows. Der Image-Workflow baut PRs ohne Push. Auf main startet er erst nach erfolgreichem `CI`-Push-Lauf derselben SHA; fehlgeschlagene/abgebrochene CI startet keinen Publish-Job.

Zuerst entstehen App-Tags `sha-<volle SHA>-<Run-ID>-<Attempt>`; OCI-Labels enthalten Revision, Version, Commitzeit und Source-URL. Die zusätzlichen Run-Felder verhindern, dass ein erneuter Build derselben SHA einen vorherigen Build überschreibt. Die Promotion prüft CI erneut und prüft **alle drei** Versionsdigests vor dem ersten Schreibvorgang. Ein bestehender Versionsdigest muss exakt identisch sein. Ein vorhandener Git-Release-Tag mit anderer SHA oder inzwischen verschobenes main führt zu einem reinen Kandidatensatz ohne Version-Promotion. Der Build-/Promotionslauf schreibt kein `latest`. Erst der erfolgreiche `Release`-Workflow setzt für API, Web, Media und MinIO den Alias `latest` auf die Digests des veröffentlichten stabilen Releases. Zuvor prüft er den ganzen Satz, den Git-Tag, CI und Publish-Lauf; ältere Releases und Kandidaten dürfen die Aliase nicht zurücksetzen. MinIO behält seinen separaten Pin; dessen `latest` zeigt auf den MinIO-Digest aus dem Release-Satz.

Das Artifact `image-set` enthält:

- `image-set.json`: Revision, Version, CI-/Publish-Run, Promotionsstatus, API/Web/Media-Digests und separaten MinIO-Pin/Digest.
- `image-set.env`: vier `GELABBER_*_IMAGE=ghcr.io/…@sha256:…`-Referenzen für Compose.

Kandidatensätze sind **keine Releases**. Ein abgeschlossener Hotfix bekommt erst nach koordinierter Abnahme eine neue Workspace-/Compose-Version. Der Koordinator startet danach den Workflow `Release` **auf main** mit passendem `tag` und erfolgreicher `image_run_id`. Dieser prüft Manifest, CI, aktuellen main-Stand und Versionsdigests erneut, bewahrt existierende Tags/Releases und hängt den Satz an den neuen GitHub-Release. Es gibt keinen automatischen Release auf jedem Push und keinen Release von Feature-/Major-Zwischenständen. Kein Trigger durch beliebige `v*`-Tag-Pushes.

Falls ein Alias-Update unterbrochen wurde oder bei einem älteren Release fehlte: `Release` auf **main** mit dessen `tag` und `image_run_id` starten und **refresh_latest_only** aktivieren. Das repariert ausschließlich die Aliase des aktuell als latest veröffentlichten stabilen GitHub-Releases, ohne Builds, neue Releases oder Änderungen an festen Tags. Für v0.2.5: `tag=v0.2.5`, `image_run_id=36549037951`. Alle vier Aliase werden einzeln geschrieben; bei einem Fehler bleibt der Workflow rot und kann erneut gestartet werden.

## Rollout und Rollback mit exakten Digests

Den Satz nach Abnahme aus dem freigegebenen Release herunterladen und dauerhaft beim Deployment archivieren. Für die erste Migration den **tatsächlich laufenden** bisherigen Satz sichern: `docker compose images` / Container-Inspect und Registry-Digests erfassen; nicht annehmen, dass ein alter mutable Tag dem laufenden Container entspricht. Diesen alten Satz als `previous.env` speichern (alle vier Image-Variablen). Zugangsdaten bleiben ausschließlich in `.env`, nicht im Manifest.

```bash
cd deploy/compose
# Freigegebenes image-set.env hier als next.env ablegen.
# .env enthält wie bisher Secrets und ggf. COMPOSE_FILE für Homelab.
docker compose --env-file .env --env-file next.env config -q
docker compose --env-file .env --env-file next.env config --images
# Backup gemäß Abschnitt Backup; alte Manifest-/Env-Dateien behalten.
docker compose --env-file .env --env-file next.env pull --policy always api web media minio
docker compose --env-file .env --env-file next.env up -d --no-deps --no-build --pull never --force-recreate api web media minio
# Anschließend: /ready, /media/ready, Login, Upload und echte Medien/Relay-Abnahme.
```

`--no-build` verhindert einen unbemerkten lokalen Ersatzbuild, der explizite Pull löst das bisherige `pull_policy: missing`-Problem. Bereits im Shell-Environment exportierte `GELABBER_*_IMAGE`-Variablen vorher entfernen, da sie Env-Dateien übersteuern. Homelab behält sein `COMPOSE_FILE`; alternativ dieselben `-f`-Overlays bei **allen** Befehlen verwenden. Images werden als Satz vorab geladen; Containerwechsel sind nicht atomar und benötigen ein Wartungsfenster.

Bei fehlgeschlagener Abnahme den archivierten vorherigen Digest-Satz verwenden:

```bash
docker compose --env-file .env --env-file previous.env config -q
docker compose --env-file .env --env-file previous.env pull --policy always api web media minio
docker compose --env-file .env --env-file previous.env up -d --no-deps --no-build --pull never --force-recreate api web media minio
```

Kein `down -v`, keine Volumes löschen oder neu benennen: Postgres-/MinIO-Daten bleiben an denselben Volumes. Ein Image-Rollback ersetzt **keinen** Datenbank-Restore; vor Releases müssen Migrationen auf Rückwärtskompatibilität geprüft und Backups erstellt werden. Nach inkompatiblen Migrationen ist der separat geprüfte Restore erforderlich. Readiness alleine ist keine Medien-/Storage-Abnahme.

Die Pipeline verhindert Überschreiben innerhalb dieser Workflows durch gemeinsamen Promotions-/Release-Lock und Digest-Prüfung. GHCR bietet damit keine transaktionsweite Unveränderlichkeit gegen externe Maintainer-Pushes. Bei abgebrochener Promotion können einzelne neue Versionstags bereits existieren; der Lauf liefert dann keinen freigegebenen Satz, und `Release` bleibt gesperrt. Wiederholungsbuilds mit anderen Digests werden ebenfalls gesperrt, statt Tags zu reparieren/überschreiben. CI-/Artifact-Aufbewahrung ist 90 Tage; freigegebene Sätze zusätzlich dauerhaft archivieren.

Lokale Prüfung für Auftrag09: [Belege und Grenzen](stability-09-evidence.md). Buildidentität ist über OCI/Manifest verfügbar; Health-Responses und Produktquellcode wurden nicht verändert. Compose-/Browser-Smoke-Tests laufen lokal: nach dem Start des lokalen Stacks in `web` mit `npm run test:browser-smoke` beziehungsweise `npm run test:e2e-app`. Der Compose-Smoke-Job wurde aus GitHub Actions entfernt; die reguläre CI prüft Rust, Frontend und Deployment-Konfiguration.
