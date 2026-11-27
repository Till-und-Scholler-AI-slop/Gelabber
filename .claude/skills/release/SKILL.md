---
name: release
description: Prepare and ship a Gelabber release vX.Y.Z from the development line to main, then complete the Linear milestone and sync main back.
disable-model-invocation: true
---

# Release vX.Y.Z

The rules are in AGENTS.md ("Branches and pull requests", "Planning"). This
is the order of the work; check each step's result before the next one.

1. **Scope.** The Linear milestone vX.Y.Z has no open issue that should
   ship; move the rest to the next milestone. CI on the development line's head is
   green, and so is the last Extended E2E push run on it (its issue "Extended
   E2E fails on vX.Y" is closed).
2. **Branch.** `release/vX.Y.Z` from the development line, then
   `git merge origin/main`, so hotfixes on `main` stay in.
3. **Version.** Bump the same files as the last release commit:
   `git show --stat "$(git log origin/main --grep '^release: v' -1 --format=%h)"`
   (workspace manifests and lock files, `web/src/version.ts`, the image tags
   in `deploy/compose/compose.yaml`, `deploy/README.md`). Then
   `python3 -m unittest discover -s deploy/compose/tests` (it checks that the
   versions agree).
4. **Notes.** `.github/release-notes/vX.Y.Z.md`, short, in the shape of
   `v0.12.0.md` (notes before it are longer and follow an older shape):
   - `# Gelabber vX.Y.Z`, then two or three sentences on what the release
     is about, with the upgrade risk in bold (migrations, backup).
   - `## Changes`: one line per change, grouped under a few bold topics,
     saying what changed for people using it and ending with its PRs and
     Linear IDs, e.g. `#651 (GEL-27)`. A change without a PR names its
     commit. The detail stays in the PR.
   - `## Upgrade notes`: backup, migrations by number, new or changed
     `.env` variables, image pins, rollback, order (server, then clients)
     and what breaks older clients.
   - A last line linking the compare view `vPREV...vX.Y.Z`.

   The notes describe what ships. Pending tests and follow-ups go into
   Linear issues; the notes have no "Open" section. Source:
   `git log --first-parent --format='%h %s' <last tag>..HEAD`. Check the
   upgrade traps that came up before: image pins that must move together,
   new required `.env` variables, and migrations that cannot be undone.
5. **Pull request** `release: vX.Y.Z` to `main`. A maintainer merges it once
   `ci-ok` is green; no second approval is needed. Extended E2E runs on it by
   itself; a red run there is understood before the merge. **Merging it deploys
   production:** `release.yml` moves `latest` and the server's Watchtower
   pulls it. So the merge needs an explicit go from a maintainer, never only
   an agent's decision, and before it the operator takes a backup
   (`deploy/backup.sh`, see `deploy/README.md` "Backup") or pauses
   Watchtower. With a migration in the notes, rollback is only through that
   backup. Until the release is out, nothing else is merged into `main`.
6. **After the merge** the chain runs by itself: CI (push) on `main`, then
   "Publish app images", then `release.yml`, which tags, publishes and starts
   `desktop-release.yml` for the tag. That run signs and attaches the apps
   the release PR's CI packaged (its job "Packages of the release's pull
   request" names the run); if `main` moved or the packages expired after 14
   days, it builds them again, which takes about 16 minutes more. The release becomes "Latest" only
   when that run's `mark-latest` job has attached the apps: until then the
   installed apps and the web's download links keep the previous one. If the
   desktop run fails, fix it and run it again for the tag. Watch it with
   `gh run list --branch main --limit 5`. If CI on the release commit was
   cancelled, re-run it on `main`'s head. `release.yml` by hand
   (`-f tag=vX.Y.Z -f image_run_id=<publish run>`) only repairs a release.
   Never `gh release create`.
7. **Desktop packages.** The pacman repository job publishes to GitHub
   Pages without an approval. The `libwebrtc-packages` and `build-deps` releases stay
   pre-releases: as "Latest", the apps' updater and the web client's
   download links would point at them.
8. **Planning.** In Linear, complete the milestone, move its open issues to
   the next one, and write the next milestone's goal if it has none.
9. **Sync** `main` into the development line: branch `sync/main-into-vX.Y`,
   PR to the development line, keeping the release commits.

Tell the person what operators have to do from the notes' upgrade section.
