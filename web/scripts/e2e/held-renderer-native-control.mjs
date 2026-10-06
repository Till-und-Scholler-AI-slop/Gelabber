/* global process, window */
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import { URL } from "node:url";
import {
  startHarness,
  click,
  observe,
  check,
  CheckFailure,
} from "./harness.mjs";
import { nativeEvaluate } from "./native-evaluate.mjs";
import { progress } from "./media.mjs";
import { holdActiveMedia, heldMedia, closeAccessActors } from "./access.mjs";

// Instrument negative control, not a revocation acceptance scenario.
const id = "held-renderer-detects-progress-without-native-rtp-reports";
if (process.env.GELABBER_E2E_CASES && process.env.GELABBER_E2E_CASES !== id)
  throw new Error("conflicting native renderer control selection");
process.env.GELABBER_E2E_CASES = id;
const source = await readFile(new URL("./access.mjs", import.meta.url), "utf8");
const stopCheck = source
  .slice(source.indexOf("channel-server-delete-active-sockets"))
  .match(/check\(\s*after\.frames === settled\.frames[\s\S]*?\n\s*\);/)[0];
const h = await startHarness();
try {
  await h.run(id, [], async () => {
    const f = await h.fixture();
    try {
      await click(f.owner, "Beitreten");
      await click(f.owner, "Go Live");
      await click(f.watcher, "Zuschauen");
      await progress(f.watcher);
      const positive = await holdActiveMedia(f.watcher, { renderer: true });
      const hidden = await nativeEvaluate(f.watcher, async function sample() {
        const state = window.__e2e;
        let before = 0,
          after = 0;
        for (const peer of state.heldPeers) {
          const original = peer.getStats.bind(peer);
          const isVideo = (r) =>
            r.type === "inbound-rtp" && (r.kind ?? r.mediaType) === "video";
          before += [...(await original()).values()].filter(isVideo).length;
          peer.getStats = async (...args) =>
            new Map(
              [...(await original(...args)).entries()].filter(
                ([, r]) => !isVideo(r),
              ),
            );
          state.restoreClose.push(() => {
            peer.getStats = original;
          });
          after += [...(await peer.getStats()).values()].filter(isVideo).length;
        }
        return {
          nativeVideoReportsBefore: before,
          nativeVideoReportsHidden: after,
        };
      });
      check(
        hidden.nativeVideoReportsBefore > 0 &&
          hidden.nativeVideoReportsHidden === 0,
        "native-rtp-report-mask-not-exercised",
        hidden,
      );
      const settled = await observe(500, () => heldMedia(f.watcher));
      const after = await observe(1_000, () => heldMedia(f.watcher));
      check(
        after.frames > settled.frames + 3,
        "masked-rtp-renderer-did-not-observe-real-progress",
        { positive, hidden, settled, after },
      );
      let stopRejected = false;
      try {
        // Isolate the actual frame-stop condition. Other outcomes are synthetic
        // denied/closed controls here; no account was revoked.
        vm.runInNewContext(stopCheck, {
          check,
          settled,
          after: { ...after, openSockets: 0 },
          fresh: { status: 404 },
          held: { accepted: false },
          scope: "instrument-control",
        });
      } catch (error) {
        if (!(error instanceof CheckFailure)) throw error;
        stopRejected = true;
      }
      check(stopRejected, "continuing-native-renderer-became-false-stop");
      return {
        instrumentControlOnly: true,
        fixtureAccounts: 3,
        fixtureServers: 1,
        sourceBinding: "unchanged held native receiver track",
        instrumentFault:
          "video RTP stats deliberately hidden while authorized native media continues",
        boundarySideConditions:
          "synthetic denied/closed values isolate the real frame-stop check; no revocation",
        positive,
        hidden,
        settled,
        after,
        stopRejected,
      };
    } finally {
      await closeAccessActors([f.owner, f.member, f.watcher], [f.watcher]);
    }
  });
} finally {
  await h.finish();
}
