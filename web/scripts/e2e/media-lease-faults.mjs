import { nativeEvaluate, NativeInterfaceFailure } from "./native-evaluate.mjs";
import { attemptAll } from "./teardown.mjs";
import {
  check,
  click,
  navigate,
  snapshot,
  until,
  observe,
} from "./harness.mjs";
import { progress } from "./media.mjs";
import {
  heldMedia,
  holdActiveMedia,
  releaseHeld,
  closeAccessActors,
} from "./access.mjs";
/* global window */
export async function closeLeaseFixture(h, f, restored) {
  const failures = await attemptAll([
    ...(!restored
      ? [
          ["api-resume", () => h.faultRuntime.resumeApi()],
          ["redis-restore", () => h.faultRuntime.redis.restore()],
        ]
      : []),
    [
      "held-restore-and-context-close",
      () => closeAccessActors([f.owner, f.member, f.watcher], [f.watcher]),
    ],
  ]);
  if (failures.length)
    throw new NativeInterfaceFailure({
      stage: "lease-owned-restore",
      failedSteps: failures,
      nativeDataAvailable: false,
    });
}
export async function leaseFaultScenarios(h, options) {
  const id = "api-redis-outage-live-lease-recovery";
  if (!h.faultRuntime?.media) {
    h.blocked(
      id,
      "requires owned API and approved SFU attached to own Redis proxy",
      ["05b", "03b", "08b"],
    );
    return;
  }
  await h.run(id, ["05b", "03b", "08b"], async () => {
    const controls = [];
    for (const plane of ["api", "redis"]) {
      const f = await h.fixture();
      const channel = f.voicePath.split("/").at(-1);
      let restored = false;
      try {
        await nativeEvaluate(
          f.member,
          (channel) => {
            window.__e2e.expectedVoiceChannel = channel;
          },
          channel,
        );
        await click(f.owner, "Beitreten");
        await click(f.owner, "Go Live");
        await click(f.watcher, "Zuschauen");
        const initial = await progress(f.watcher, options);
        check(
          (await h.faultRuntime.liveExists(channel, f.owner.id)) === 1,
          "fixture-positive-live-lease-missing",
        );
        const before = await until(
          () => snapshot(f.member),
          (s) => s.liveOccupancy === 1 && s.voiceOccupancy === 1,
          "fixture-positive-passive-roster-missing",
        );
        await holdActiveMedia(f.watcher);
        const faultAt = Date.now();
        if (plane === "api") await h.faultRuntime.pauseApi();
        else h.faultRuntime.redis.block();
        const closed = await until(
          () => heldMedia(f.watcher),
          (s) => s.openSockets === 0,
          "authority-fault-did-not-close-native-media",
          plane === "api" ? 5_000 : 3_000,
        );
        const closeObservedMs = Date.now() - faultAt;
        //8s covers the existing5s seat/live TTL, without a Redis key mutation.
        const stopped = await observe(
          Math.max(0, 8_000 - (Date.now() - faultAt)),
          () => heldMedia(f.watcher),
        );
        const tail = await observe(1_000, () => heldMedia(f.watcher));
        check(
          tail.frames === stopped.frames,
          "authority-fault-kept-native-decoded-media",
          { controlConfirmed: true, plane, stopped, tail },
        );
        check(
          (await h.faultRuntime.liveExists(channel, f.owner.id)) === 0,
          "expired-live-lease-still-held",
          { controlConfirmed: true, plane },
        );
        if (plane === "api") h.faultRuntime.resumeApi();
        else h.faultRuntime.redis.restore();
        restored = true;
        const restoredAt = Date.now();
        const empty = await until(
          () => snapshot(f.member),
          (s) =>
            s.voiceRosterSnapshots > before.voiceRosterSnapshots &&
            s.liveOccupancy === 0 &&
            s.voiceOccupancy === 0,
          "passive-roster-not-corrected-after-authority-recovery",
          20_000,
        );
        await releaseHeld(f.watcher);
        // Explicit fresh voice join / Live start by B proves the expired slot can
        // be occupied by a different owner; A's old claim cannot block or renew it.
        await click(f.member, "Beitreten");
        await click(f.member, "Go Live");
        await navigate(f.watcher, f.voicePath, f.base);
        await click(f.watcher, "Zuschauen");
        const resumed = await progress(f.watcher, {
          ...options,
          budget: 20_000,
        });
        await nativeEvaluate(
          f.watcher,
          (publisher) => {
            window.__e2e.expectedAudioPeer = window.__e2e.peers.find(
              (p) => p.connectionState === "connected",
            );
            window.__e2e.expectedLivePublisher = publisher;
          },
          f.member.id,
        );
        const freshSource = await until(
          () => snapshot(f.watcher),
          (s) =>
            s.watchVideoSources.selectedLive === 1 &&
            s.watchVideoSources.foreign === 0,
          "fresh-live-selected-different-owner-not-confirmed",
        );
        check(
          (await h.faultRuntime.liveExists(channel, f.owner.id)) === 1,
          "rejoined-owner-did-not-obtain-fresh-live-lease",
          { controlConfirmed: true, plane },
        );
        controls.push({
          plane,
          initial,
          before,
          closeObservedMs,
          closed,
          stopped,
          tail,
          empty,
          recoveryMs: Date.now() - restoredAt,
          resumed,
          freshSource,
          oldLeaseExpired: true,
          freshDifferentOwnerClaim: true,
        });
      } finally {
        // These faults affect only captured own child/proxy references.
        await closeLeaseFixture(h, f, restored);
      }
    }
    return {
      controls,
      sourceBounds: {
        seatLiveTtlMs: 5_000,
        authorityIntervalMs: 1_000,
        redisDeadlineMs: 500,
      },
      limitation:
        "observed poll timestamps are not intrusive SFU task/publication metrics",
    };
  });
}
