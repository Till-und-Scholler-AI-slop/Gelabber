# Auftrag09 – Belege und Übergabe

Stand: 28.09.2026. Baseline `828ee2341e57a7e77a971b1f9c52d3372f046fb8`, Branch `fix/stability-09`, PR-Ziel main. Kein Push, Release-Tag oder Produktionsrollout. Geändert wurden ausschließlich Workflows samt Gate-Helfer, Dockerfiles, Compose, Deploy-Tests und Deploy-Dokumentation. Produktquellcode, Stack-Pins und Browser-Smoke bleiben unverändert.

## Repro und Änderungen

Vorheriger Codebefund: `shared/**` fehlt in CI-/Image-Filtern; der Main-Push veröffentlicht sofort Workspace-Version und `latest`; `Release` wartet erst danach auf CI. Neue Main-SHA mit Version 0.2.4 überschreibt somit das ausgelieferte Image. MinIO schreibt ebenfalls App-Version und `latest`. CI lädt Caddy ohne Version, obwohl Compose 2.11.4 verwendet.

Nachher:

- PR-Filter erfassen shared, sämtliche Rust-Workspace-Mitglieder, Web-Kontext, Docker-Helfer, Dockerignore, Cargo-Pins/-Lockfile und alle Workflows. Main-CI ist ungefiltert, damit jede SHA einen exakten Gate-Lauf erhält.
- App-Publishing startet nur aus erfolgreichem Main-Push-CI derselben SHA. PR-Builds pushen nicht. Erster Publish nur unter SHA + Run-ID + Attempt; keine Überschreibung beim Rebuild derselben Revision.
- Promotion prüft den vertrauenswürdigen CI-Lauf erneut. Drei Versionsdigests werden vor dem ersten Schreibvorgang geprüft; andere Digests werden abgelehnt. Bestehender Release-Tag an anderer SHA oder inzwischen verschobenes main erzeugt nur einen Kandidatensatz. `latest` bleibt unverändert.
- OCI-Labels verbinden Revision, Version, Commitzeit und Source-URL mit den Images. `image-set.json` und `image-set.env` zeichnen API/Web/Media sowie den bestehenden separaten MinIO-Digest auf.
- MinIO publiziert nur seinen eigenen fehlenden Source-Pin. Bestehender Pin und historische Version-/latest-Aliase werden bewahrt. Compose verwendet diesen vorhandenen Source-Pin unabhängig von der App-Version.
- Release ist jetzt explizites `workflow_dispatch` auf main mit Publish-Run-ID. Es prüft CI, Promotionsstatus, aktuelle Main-SHA, Git-Tag und alle App-Versionsdigests; keine automatische Freigabe jeder neuen Main-Revision und kein beliebiger Tag-Trigger. Manifest-Dateien sind Release-Assets. Promotions- und Release-Workflow teilen denselben Concurrency-Lock.
- Compose unterstützt vier `GELABBER_*_IMAGE`-Digest-Overrides. Dokumentierter Pull/Recreate- und Rückweg behalten Postgres-/MinIO-Volumes. Bestehende App-Defaults bleiben v0.2.4.
- CI-Caddy-URL enthält `version=v2.11.4`; ein Versionsvergleich verwirft abweichende Downloads. Kein Dependency-Pin geändert.

## Tatsächlich ausgeführt

| Prüfung | Ergebnis |
| --- | --- |
| `PATH=/tmp/gelabber-caddy-ci/bin:$PATH python3 -m unittest discover -s deploy/compose/tests -v` | PASS: 24 Tests, keine Skips. 7 bestehende Caddy-Tests, 2 echte Compose-Konfigurationsprüfungen, 15 neue statische/gemockte Workflow-Prüfungen. |
| Download exakt derselben CI-URL | PASS: heruntergeladenes Binary meldet `v2.11.4 h1:XKxkMTgNSizEvKG6QHue6cAsFOteU2qA61w2tKkCWi0=`. Tests verwenden dieses Binary. |
| `caddy validate --config deploy/compose/Caddyfile --adapter caddyfile` | PASS mit dem heruntergeladenen Caddy 2.11.4. Homelab-Adapt-/Proxy-Vertrag durch die 7 bestehenden Tests geprüft. |
| Compose `config -q` mit Base-/Homelab-Beispiel | PASS, Docker Compose 5.5.1. |
| Digest-Rollout-/Rollback-Konfiguration | PASS für Base und Homelab: vier Digest-Referenzen wechseln b… → c…; persistente Volume-Namen und Mounts bleiben identisch. Reproduzierbar in `test_digest_compose.py`; keine realen Images hinter diesen Testdigests. |
| actionlint 1.7.12, alle vier Workflows | PASS (`-shellcheck=`; ShellCheck ist nicht installiert). |
| YAML-Parsing + `bash -n` | PASS für 27 Run-Schritte; GitHub-Ausdrücke durch Literale ersetzt. Keine Ausführung der Publish-/Release-Befehle. |
| Python `py_compile`, `git diff --check` | PASS. |
| Lesende GHCR-/Buildx-Prüfung | Bestehender MinIO-Pin abrufbar: `sha256:528e6b8ceff5c806ff4077987a09b1cee7e6b965dbc5caad94114af60b95c79e`. Buildx 0.37.0 unterstützt den verwendeten Digest-Formatter. |
| Lesende Missing-Tag-Prüfung | GHCR/Buildx liefert `ERROR: <exakte Referenz>: not found`; genau diese Antwort und `manifest unknown` erlauben Erstellung. Auth-/Timeout-/generische Repository-Fehler werden abgelehnt. |

## Explizit gemockte Gate-Invarianten

`test_image_set.py` ersetzt GitHub-/Registry-Zugriffe; diese Ergebnisse sind **keine ausgeführten GitHub-Actions-Läufe**:

- Fehlgeschlagene, abgebrochene, ausstehende, fremde oder nicht exakt passende CI: kein Versionsschreibvorgang und kein freigegebenes Manifest.
- Shared-only-/Build-Abhängigkeitsänderung: tatsächliche Workflow-Filter enthalten den Pfad; Main-CI hat keinen Pfadfilter. Trigger nicht remote ausgelöst.
- Main-Commit mit unveränderter bereits getaggter Version an anderer SHA: Manifest `promoted=false`, null Release-Tag-Schreibvorgänge.
- Anderer API/Web/Media-Versionsdigest: gesamter Preflight scheitert vor jedem Versionsschreibvorgang.
- Erfolgreiche Promotion: drei exakte Digest-Quellen mit `--prefer-index=false`, nachfolgende Digest-Gleichheit. Diese Flag-Semantik ist in der [offiziellen Buildx-Dokumentation](https://docs.docker.com/reference/cli/docker/buildx/imagetools/create/) beschrieben.
- Release verweigert Kandidatensatz, fehlgeschlagenen Publisher, fremden Git-Tag und andere Versionsdigests; bestätigter Satz wird als Release-Asset-Vertrag übergeben, ohne echten Release zu erstellen.

## Grenzen und nächste Integration

Docker-Daemonzugriff bleibt verweigert. Keine lokalen App-/MinIO-Dockerbuilds, Containerstarts, echten Registry-Pushes/Promotions, tatsächlichen Container-/Datenbank-Rollbacks oder neue Remote-CI-Läufe. Rust-/Web-Produkttests wurden hier nicht erneut ausgeführt: Produktquellcode unverändert; bestehender kompletter CI- und Compose-/Browser-/TURN-Smoke bleibt der Remote-Gate. Keine Live-Medien-/Upload-Abnahme und keine Behauptung, dass die laufende Instanz veraltet sei.

Versionstags sind innerhalb dieser Workflows durch Preflight und gemeinsamen Lock geschützt, aber GHCR-Schreibrechte außerhalb der Pipeline sind kein atomarer Registry-Schutz. Drei Tags können nicht als Transaktion geschrieben werden: Abbruch während Promotion kann einen Teilsatz hinterlassen; der fehlgeschlagene Lauf liefert keinen freigegebenen Satz und wird vom Release-Gate abgelehnt. Ein Rebuild mit abweichenden Digests wird gesperrt; keine automatische Tag-Reparatur. Der getrennte MinIO-Pin muss vorhanden/lesbar sein, sonst scheitert die Manifest-Erzeugung.

Reproduzierbarkeit bedeutet hier nachvollziehbare Revision plus archivierte exakte Image-Digests, keine behauptete bitidentische Neukompilierung (bestehende apt-/Registry-/Caddy-Download-Buildquellen bleiben bestehen). Health-Responses wurden nicht um Revisionen erweitert; OCI/Manifest liefern die Identität ohne Eingriff in fremden Produktquellcode.

Image-Rollback erhält Volume-Identitäten; nach inkompatiblen DB-Migrationen braucht es einen geprüften Backup-/Restore-Plan. Rollout ist nicht atomar und braucht ein Wartungsfenster. Artifact-Aufbewahrung 90 Tage, freigegebene Sätze dauerhaft archivieren.

Koordinator übernimmt unabhängiges Review, Remote-CI, Push/PR und Integration. Spätere finalisierte Media-Lifecycle- und Web-Auth-Browserprüfungen können im Compose-Job nach Chromium-Installation und Stack-Readiness ergänzt werden; heutiger Smoke und fremde Skripte wurden nicht bearbeitet. Für einen neuen freigegebenen Hotfix müssen Workspace-/Compose-Version nach Gesamt-/Teilabnahme gemeinsam erhöht werden; 09 selbst erstellt keinen neuen Release.
