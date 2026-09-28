import { nativeEvaluate } from "./native-evaluate.mjs";
/* global window */
import {
  CheckFailure,
  check,
  click,
  snapshot,
  until,
  observe,
} from "./harness.mjs";
import { activePeers, progress } from "./media.mjs";
const audioPackets = (s) =>
  activePeers(s)
    .flatMap((p) => p.inbound)
    .filter((r) => r.kind === "audio")
    .reduce((n, r) => n + r.packets, 0);
const audioSenders = (s) =>
  activePeers(s)
    .flatMap((p) => p.audioSenders)
    .filter((t) => t.live);

export async function concurrentClaim(h, f, { reset, options }) {
  await reset(f);
  for (const actor of [f.owner, f.member]) {
    await click(actor, "Beitreten");
    await until(
      () => snapshot(actor),
      (s) => activePeers(s).some((p) => p.connection === "connected"),
      "fixture-claim-voice-not-connected",
    );
    await nativeEvaluate(actor, () => {
      window.__e2e.holdLiveClaims = true;
    });
  }
  try {
    // Both actual UI attempts are prepared before either native claim is transmitted.
    await Promise.all([click(f.owner, "Go Live"), click(f.member, "Go Live")]);
    const held = await Promise.all(
      [f.owner, f.member].map((actor) =>
        until(
          () => snapshot(actor),
          (s) => s.heldLiveClaimCount === 1,
          "fixture-concurrent-live-claim-not-held",
        ),
      ),
    );
    await Promise.all(
      [f.owner, f.member].map((actor) =>
        nativeEvaluate(actor, () => {
          const state = window.__e2e;
          state.holdLiveClaims = false;
          for (const send of state.heldLiveClaims.splice(0)) send();
        }),
      ),
    );
    const settled = await until(
      async () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
      (states) =>
        states.filter((s) => s.captures.some((c) => c.state === "live"))
          .length === 1,
      "concurrent-live-claim-not-exclusive",
    );
    const winnerIndex = settled.findIndex((s) =>
      s.captures.some((c) => c.state === "live"),
    );
    const loser = [f.owner, f.member][1 - winnerIndex];
    const loserState = await observe(3_000, () => snapshot(loser));
    check(
      loserState.captures.every((c) => c.state !== "live") &&
        activePeers(loserState)
          .flatMap((p) => p.outbound)
          .filter((r) => r.kind === "video")
          .every((r) => !r.frames && !r.bytes),
      "concurrent-claim-loser-published-or-leaked",
      { controlConfirmed: true, held, settled, loser: loserState },
    );
    await click(f.watcher, "Zuschauen");
    return {
      simultaneousPreparedClaims: 2,
      held,
      loser: loserState,
      winner: await progress(f.watcher, options),
    };
  } finally {
    for (const actor of [f.owner, f.member])
      await nativeEvaluate(actor, () => {
        window.__e2e.holdLiveClaims = false;
        window.__e2e.heldLiveClaims.length = 0;
      });
  }
}

export async function mediaExtraScenarios(h, f, { begin, reset, options }) {
  await h.run("voice-only-duplex-single-source", ["02", "08b"], async () => {
    await reset(f);
    for (const [actor, publisher] of [
      [f.owner, f.member],
      [f.member, f.owner],
    ]) {
      await click(actor, "Beitreten");
      await nativeEvaluate(
        actor,
        (publisher) => {
          const state = window.__e2e;
          state.expectedAudioPublishers = [publisher];
        },
        publisher.id,
      );
    }
    const before = await until(
      async () => {
        for (const actor of [f.owner, f.member])
          await nativeEvaluate(actor, () => {
            window.__e2e.expectedAudioPeer = window.__e2e.peers.find(
              (p) => p.connectionState === "connected",
            );
          });
        return Promise.all([snapshot(f.owner), snapshot(f.member)]);
      },
      (states) => states.every((s) => audioPackets(s) > 0),
      "fixture-voice-only-duplex-audio-missing",
      options.budget,
    );
    const after = await until(
      () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
      (states) =>
        states.every((s, i) => audioPackets(s) > audioPackets(before[i]) + 3),
      "voice-only-duplex-audio-did-not-progress",
      options.budget,
    );
    for (const s of after) {
      check(
        s.roomAudio.perSource.length === 1 &&
          s.roomAudio.perSource[0] === 1 &&
          s.roomAudio.foreign === 0 &&
          !s.duplicateAudioPlaybackTracks &&
          s.displayCalls === 0 &&
          s.cameraCalls === 0 &&
          audioSenders(s).length === 1 &&
          audioSenders(s)[0].enabled,
        "voice-only-source-duplicate-or-unexpected-capture",
        { before, after },
      );
      const paths = activePeers(s).flatMap((p) => p.selected);
      check(
        paths.length > 0 &&
          paths.every(
            (p) =>
              p.state === "succeeded" &&
              (options.relay
                ? p.local === "relay"
                : p.local !== "relay" && p.remote !== "relay"),
          ),
        "selected-network-path-mismatch",
        { paths },
      );
    }
    return {
      before,
      after,
      limitation: "native duplex transport, not human intelligibility",
    };
  });
  await h.run(
    "forced-track-arrival-reorder-source-identity",
    ["01", "08b"],
    async () => {
      await begin(f);
      await progress(f.watcher, options);
      await nativeEvaluate(f.member, () => {
        window.__e2e.holdTracks = true;
      });
      try {
        await click(f.owner, "Kamera an");
        await click(f.owner, "Bildschirm teilen");
        await click(f.member, "Beitreten");
        const held = await until(
          () => snapshot(f.member),
          (s) => s.heldVideoTracks === 3,
          "fixture-three-native-video-arrivals-not-held",
          20_000,
        );
        // Reverse native callback delivery only; the actual peers/RTP/SDP stay native.
        const kinds = await nativeEvaluate(f.member, () => {
          const state = window.__e2e;
          state.holdTracks = false;
          const pending = state.heldTracks.splice(0).reverse();
          const kinds = pending.map((item) => item.event.track.kind);
          for (const item of pending) item.deliver();
          return kinds;
        });
        return {
          held,
          callbackOrder: "reversed",
          deliveredKinds: kinds,
          live: await progress(f.member, options),
          screen: await progress(f.member, {
            ...options,
            kind: "screen",
            color: [30, 220, 30],
          }),
          camera: await progress(f.member, {
            ...options,
            kind: "camera",
            color: [30, 60, 220],
          }),
        };
      } finally {
        await nativeEvaluate(f.member, () => {
          window.__e2e.holdTracks = false;
          for (const item of window.__e2e.heldTracks.splice(0)) item.deliver();
        });
      }
    },
  );
  await h.run(
    "rejected-native-sdp-keeps-voice-other-source-no-ghost",
    ["02", "08b"],
    async () => {
      await reset(f);
      await click(f.owner, "Beitreten");
      await click(f.member, "Beitreten");
      await click(f.owner, "Bildschirm teilen");
      await click(f.owner, "Kamera an");
      const warmupStart = Date.now();
      const cameraFixtureWarmup = { deadlineMs: 20_000, status: "FAIL" };
      let camera, existingScreen, before;
      try {
        const ready = await until(
          () => snapshot(f.member),
          (s) =>
            s.videos.some(
              (v) => v.kind === "camera" && v.width === 640 && v.height === 360,
            ),
          "fixture-sdp-camera-warmup-deadline",
          cameraFixtureWarmup.deadlineMs,
        );
        Object.assign(cameraFixtureWarmup, {
          status: "PASS",
          WarmupMs: Date.now() - warmupStart,
          width: ready.videos.find((v) => v.kind === "camera").width,
          height: ready.videos.find((v) => v.kind === "camera").height,
        });
        // Only this positive fixture setup precedes the unchanged five-second
        // progress check. No warmup is allowed after the SDP fault.
        camera = await progress(f.member, {
          ...options,
          budget: 5_000,
          kind: "camera",
          color: [30, 60, 220],
        });
        await until(
          () => snapshot(f.member),
          (s) => audioPackets(s) > 0,
          "fixture-pre-sdp-audio-not-received",
        );
        existingScreen = await progress(f.member, {
          ...options,
          kind: "screen",
        });
        before = await snapshot(f.owner);
      } catch (error) {
        cameraFixtureWarmup.WarmupMs ??= Date.now() - warmupStart;
        const measured = error.metrics?.last?.videos?.find(
          (v) => v.kind === "camera",
        );
        if (measured)
          Object.assign(cameraFixtureWarmup, {
            width: measured.width,
            height: measured.height,
          });
        if (error instanceof CheckFailure) {
          error.metrics = {
            ...error.metrics,
            cameraFixtureWarmup,
            faultExercised: false,
          };
          throw error;
        }
        throw new CheckFailure("fixture-sdp-positive-control-interface-error", {
          cameraFixtureWarmup,
          faultExercised: false,
        });
      }
      await nativeEvaluate(f.owner, () => {
        window.__e2e.rejectNextVideoAnswer = true;
      });
      try {
        await click(f.owner, "Go Live");
        const failed = await until(
          () => snapshot(f.owner),
          (s) =>
            s.rejectedSdp === 1 &&
            s.displayCalls === before.displayCalls + 1 &&
            s.captures.filter((c) => c.kind === "display" && c.slot === 2)
              .length === 1 &&
            s.captures
              .filter((c) => c.kind === "display" && c.slot === 2)
              .every((c) => c.state === "ended"),
          "rejected-sdp-ghost-live-or-fault-unexercised",
          20_000,
        );
        check(
          audioSenders(failed).length === 1 &&
            audioSenders(failed)[0].enabled &&
            failed.micCalls === before.micCalls,
          "rejected-sdp-lost-existing-mic",
          { controlConfirmed: true, before, failed },
        );
        const afterCamera = await progress(f.member, {
          ...options,
          kind: "camera",
          color: [30, 60, 220],
        });
        const afterScreen = await progress(f.member, {
          ...options,
          kind: "screen",
        });
        check(
          !afterCamera.last.videos.some(
            (v) => v.kind === "live" && v.width > 0,
          ),
          "rejected-sdp-ghost-live-display",
          { controlConfirmed: true, after: afterCamera.last },
        );
        await click(f.owner, "Go Live");
        const retry = await progress(f.member, options);
        return {
          injection:
            "one real native setRemoteDescription rejection of malformed answer",
          cameraFixtureWarmup,
          faultExercised: true,
          before,
          failed,
          camera,
          existingScreen,
          afterCamera,
          afterScreen,
          retry,
        };
      } catch (error) {
        const observed = await snapshot(f.owner).catch(() => null);
        const diagnostics = {
          cameraFixtureWarmup,
          faultExercised: observed?.rejectedSdp > 0,
        };
        if (error instanceof CheckFailure) {
          error.metrics = { ...error.metrics, ...diagnostics };
          throw error;
        }
        throw new CheckFailure("sdp-fault-interface-error", diagnostics);
      } finally {
        await nativeEvaluate(f.owner, () => {
          window.__e2e.rejectNextVideoAnswer = false;
        });
      }
    },
  );
  await h.run(
    "session-audio-mute-deafen-volume-playback-retry",
    ["08a", "08b"],
    async () => {
      await begin(f);
      await progress(f.watcher, options);
      await click(f.member, "Beitreten");
      await until(
        () => snapshot(f.member),
        (s) => audioPackets(s) > 0,
        "fixture-bidirectional-audio-not-received",
      );
      await nativeEvaluate(
        f.watcher,
        (publishers) => {
          window.__e2e.expectedAudioPeer = window.__e2e.peers.find(
            (p) => p.connectionState === "connected",
          );
          window.__e2e.expectedAudioPublishers = publishers;
          window.__e2e.expectedLivePublisher = publishers[0];
        },
        [f.owner.id, f.member.id],
      );
      const selected = await until(
        () => snapshot(f.watcher),
        (s) =>
          s.roomAudio.perSource.length === 2 &&
          s.roomAudio.perSource.every((n) => n === 1),
        "fixture-each-room-audio-source-not-received",
      );
      check(
        selected.roomAudio.foreign === 0 &&
          selected.watchVideoSources.selectedLive === 1 &&
          selected.watchVideoSources.foreign === 0 &&
          selected.duplicateAudioPlaybackTracks === 0 &&
          selected.micCalls === 0,
        "watch-foreign-source-or-duplicate-audio",
        { selected },
      );
      // Keep Watch A while the same account joins voice B; both playback paths
      // must obey the one session's controls after SPA navigation.
      const other = await h.actor("OtherVoice");
      await f.join(other);
      let stage = "other-voice-room-navigation";
      try {
        await other.page.locator(`a[href="${f.otherPath}"]`).click();
        await click(other, "Beitreten");
        await until(
          () => snapshot(other),
          (s) =>
            activePeers(s)
              .flatMap((p) => p.outbound)
              .some((r) => r.kind === "audio" && r.packets > 0),
          "fixture-other-room-positive-audio-missing",
        );
        stage = "other-room-live-start";
        await click(other, "Go Live");
        await until(
          () => snapshot(other),
          (s) =>
            activePeers(s)
              .flatMap((p) => p.outbound)
              .some((r) => r.kind === "video" && r.frames > 0),
          "fixture-other-room-positive-live-missing",
        );
        stage = "watcher-navigation-to-other-room";
        const isolatedRoom = await observe(1_000, () => snapshot(f.watcher));
        check(
          isolatedRoom.roomAudio.perSource.length === 2 &&
            isolatedRoom.roomAudio.perSource.every((n) => n === 1) &&
            isolatedRoom.roomAudio.foreign === 0 &&
            isolatedRoom.watchVideoSources.selectedLive === 1 &&
            isolatedRoom.watchVideoSources.foreign === 0,
          "watch-other-room-source-leak",
          { controlConfirmed: true, isolatedRoom },
        );
        // Active Live adds status text to the accessible channel name. The
        // fixture's exact channel href still targets the actual SPA link.
        await f.watcher.page.locator(`a[href="${f.otherPath}"]`).click();
        await nativeEvaluate(f.watcher, () => {
          window.__e2e.rejectPlayback = true;
        });
        stage = "watcher-voice-join";
        await click(f.watcher, "Beitreten");
        await until(
          () => snapshot(f.watcher),
          (s) =>
            s.playRejected > 0 &&
            s.playback.filter((p) => p.kind === "audio" && p.audioTracks > 0)
              .length === 3,
          "fixture-three-session-audio-playback-tracks-not-present",
        );
        const retry = f.watcher.page.getByRole("button", {
          name: "Ton starten",
          exact: true,
        });
        check(
          (await retry.count()) > 0,
          "audio-playback-failure-no-visible-retry",
        );
        await nativeEvaluate(f.watcher, () => {
          window.__e2e.rejectPlayback = false;
        });
        stage = "visible-audio-retry";
        await retry.first().click();
        const audioPlays = (s) =>
          s.playback.filter((p) => p.kind === "audio" && p.audioTracks > 0);
        const playing = await until(
          () => snapshot(f.watcher),
          (s) =>
            audioPlays(s).length === 3 &&
            audioPlays(s).every((p) => !p.paused && !p.muted) &&
            !s.duplicateAudioPlaybackTracks,
          "audio-click-retry-did-not-play-every-path",
        );
        stage = "session-mic-mute";
        await click(f.watcher, "Mikrofon aus");
        const muted = await until(
          () => snapshot(f.watcher),
          (s) =>
            audioSenders(s).length === 1 &&
            audioSenders(s).every((t) => !t.enabled),
          "session-mute-did-not-disable-mic",
        );
        stage = "session-deafen";
        await click(f.watcher, "Mikrofon an");
        await click(f.watcher, "Taub stellen");
        const deafened = await until(
          () => snapshot(f.watcher),
          (s) =>
            audioPlays(s).length === 3 &&
            audioPlays(s).every((p) => p.muted) &&
            audioSenders(s).every((t) => !t.enabled),
          "session-deafen-did-not-mute-every-path",
        );
        stage = "session-undeafen";
        await click(f.watcher, "Hören");
        const hearing = await until(
          () => snapshot(f.watcher),
          (s) =>
            audioPlays(s).length === 3 &&
            audioPlays(s).every((p) => !p.muted && !p.paused) &&
            audioSenders(s).every((t) => t.enabled),
          "session-undeafen-did-not-restore-every-path",
        );
        stage = "session-volume";
        const volume = f.watcher.page.getByRole("slider", {
          name: "Wiedergabe-Lautstärke",
          exact: true,
        });
        await volume.press("Home");
        for (let i = 0; i < 35; i++) await volume.press("ArrowRight");
        const lower = await until(
          () => snapshot(f.watcher),
          (s) =>
            audioPlays(s).length === 3 &&
            audioPlays(s).every((p) => Math.abs(p.volume - 0.35) < 0.001),
          "session-volume-not-applied-to-every-path",
        );
        return {
          selected,
          isolatedRoom,
          playing,
          muted,
          deafened,
          hearing,
          lower,
          contract:
            "all same-room audio exactly once; only selected publisher Live video; no other-room sources",
          limitation:
            "native transport/playback properties only; human audible two-device quality remains manual",
        };
      } catch (error) {
        if (error instanceof CheckFailure) throw error;
        throw new CheckFailure("session-audio-interface-error", {
          stage,
          last: await snapshot(f.watcher).catch(() => null),
          other: await snapshot(other).catch(() => null),
        });
      } finally {
        await nativeEvaluate(f.watcher, () => {
          window.__e2e.rejectPlayback = false;
        });
        await other.context.close();
      }
    },
  );
}
