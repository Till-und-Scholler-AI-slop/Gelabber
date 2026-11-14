---
name: release
description: Prepare and ship a Gelabber release vX.Y.Z from the development line to main, then close the milestone and sync main back.
disable-model-invocation: true
---

# Release vX.Y.Z

The rules are in AGENTS.md ("Branches and pull requests", "Planning"). This
is the order of the work; check each step's result before the next one.

1. **Scope.** The milestone vX.Y.Z has no open issue that should ship; move
   the rest to the next milestone. CI on the development line's head is
   green.
2. **Branch.** `release/vX.Y.Z` from the development line, then
   `git merge origin/main`, so hotfixes on `main` stay in.
3. **Version.** Bump the same files as the last release commit:
   `git show --stat "$(git log origin/main --grep '^release: v' -1 --format=%h)"`
   (workspace manifests and lock files, `web/src/version.ts`, the image tags
   in `deploy/compose/compose.yaml`, `deploy/README.md`). Then
   `python3 -m unittest discover -s deploy/compose/tests` (it checks that the
   versions agree).
4. **Notes.** `.github/release-notes/vX.Y.Z.md`, in the shape of the previous
   notes: what changed for people using it, what operators must do on
   upgrade, and anything that breaks older clients. Source:
   `git log --merges --format='%s%n%b' <last tag>..HEAD`. Check the upgrade
   traps that came up before: image pins that must move together, new
   required `.env` variables, and migrations that cannot be undone.
5. **Pull request** `release: vX.Y.Z` to `main`, reviewed by a maintainer
   other than the person who asked for the release. **Merging it deploys
   production:** `release.yml` moves `latest` and the server's Watchtower
   pulls it. So the merge needs an explicit go from a person, never only an
   agent's approval, and before it the operator takes a backup
   (`deploy/backup.sh`, see `deploy/README.md` "Backup") or pauses
   Watchtower. With a migration in the notes, rollback is only through that
   backup. Until the release is out, nothing else is merged into `main`.
6. **After the merge** the chain runs by itself: CI (push) on `main`, then
   "Publish app images", then `release.yml`, which tags, publishes and starts
   `desktop-native.yml` for the tag. Watch it with
   `gh run list --branch main --limit 5`. If CI on the release commit was
   cancelled, re-run it on `main`'s head. `release.yml` by hand
   (`-f tag=vX.Y.Z -f image_run_id=<publish run>`) only repairs a release.
   Never `gh release create`.
7. **Desktop packages.** The pacman repository job waits for the
   `github-pages` environment approval. A maintainer approves it, not an
   agent. The `libwebrtc-packages` and `build-deps` releases stay
   pre-releases: as "Latest", the apps' updater and the web client's
   download links would point at them.
8. **Planning.** Close the milestone, move its open issues to the next one,
   and write the next milestone's goal if it has none.
9. **Sync** `main` into the development line: branch `sync/main-into-vX.Y`,
   PR to the development line, keeping the release commits.

Tell the person what operators have to do from the notes' upgrade section.
