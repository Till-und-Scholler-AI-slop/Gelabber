---
name: native-parity
description: Render the native desktop app's scenes and score them against the web client's screenshots with gelabber-parity. Use when a change touches what both clients show, or a parity scene in web/scripts/parity/scenes.json.
---

# Native parity

The commands and the stack's environment are in `desktop/README.md` ("Native
app", the parity part). What the README does not say:

- **libwebrtc.** `GITHUB_REPOSITORY=Till-und-Scholler-AI-slop/Gelabber
  desktop/native/scripts/libwebrtc-fetch.sh <key> <dir>` fetches the Linux
  package `desktop/native/libwebrtc-packages.sha256` lists (`<key>` is its
  name without `.tar.zst`) from the `libwebrtc-packages` release, checks its
  SHA-256 and unpacks it; point `GELABBER_LIBWEBRTC_DIR` at `<dir>`.
- **Stack.** Services through the `test-services` skill, a separate database
  for parity, the API with the README's environment, Vite on port 5180 with
  `GELABBER_DEV_PUBLIC_PAGES`, then in `web/`: `node scripts/parity/seed.mjs`
  and `node scripts/parity/shots.mjs <web-dir>`. `GELABBER_PARITY_BROWSER`
  names the Chromium binary, `GELABBER_PARITY_PSQL` the `psql` command for
  the parity database. Both shot tools and `gelabber-parity` run
  `scripts/parity/reset.mjs` before every scene (back to the seeded state)
  and `scripts/parity/say.mjs` for a scene's `before` and `after` words (a
  message from someone else, a theme); run them by hand only to repair the
  fixture.
- **One script.** Start the API, Vite, seed, shots, render and compare from
  one script run in the background (`nohup`). Processes started in an
  earlier tool call may be gone by the next one.
- **Render.** Build `gelabber-parity` once and run the binary directly, with
  `LD_LIBRARY_PATH` set to the media core's build output
  (`desktop/target/debug/build/gelabber-media-core-*/out/build`) and
  `GELABBER_FAKE_MEDIA=1`. Without a GPU, install a software Vulkan driver
  (`mesa-vulkan-drivers`).
- **Rate limit.** Shots and render sign the same users in often; a scene can
  fail with "Too many requests". Render the failed scenes again in a second
  pass.
- **Blocked downloads.** Where `codeload.github.com` is blocked, the media
  core's CMake download of rnnoise fails. Point that `FetchContent_Declare`
  at the git repository for the build, and restore the file before you
  commit.

Report each changed scene with its score from the compare report, and attach
the web and native pictures where a scene fails.
