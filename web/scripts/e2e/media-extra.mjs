import { nativeEvaluate, NativeInterfaceFailure } from "./native-evaluate.mjs";
import { attemptAll } from "./teardown.mjs";
/* global window, setTimeout, clearTimeout */
import {
  CheckFailure,
  check,
  click,
  liveTiles,
  snapshot,
  until,
} from "./harness.mjs";
import {
  activePeers,
  armPlaybackRetry,
  progress,
  waitForPath,
} from "./media.mjs";
import {
  sampleVideoSenders,
  samplePublicationFlow,
  sendTimeline,
  startSendTimeline,
} from "./probe.mjs";
import { publisherEncoderProgress } from "./publisher-encoders.mjs";
const audioPackets = (s) =>
  activePeers(s)
    .flatMap((p) => p.inbound)
    .filter((r) => r.kind === "audio")
    .reduce((n, r) => n + r.packets, 0);
const audioSenders = (s) =>
  activePeers(s)
    .flatMap((p) => p.audioSenders)
    .filter((t) => t.live);

/** Lifts the scenario's playback fault from the watcher's page, bounded:
 * a page whose native evaluation is unusable is not touched again. */
export async function restoreSessionPlayback(watcher, budgetMs = 5_000) {
  const failedSteps = await attemptAll([
    [
      "playback-restore",
      async () => {
        if (watcher.nativeEvaluationUnusable || watcher.page.isClosed?.())
          return;
        await nativeEvaluate(
          watcher,
          () => {
            window.__e2e.rejectPlayback = false;
          },
          undefined,
          budgetMs,
        );
      },
    ],
  ]);
  if (failedSteps.length)
    throw new NativeInterfaceFailure({
      stage: "session-audio-owned-restore",
      failedSteps,
      nativeDataAvailable: false,
    });
}

export async function concurrentLive(h, f, { reset, options }) {
  await reset(f);
  for (const actor of [f.owner, f.member]) {
    await click(actor, "Beitreten");
    await until(
      () => snapshot(actor),
      (s) => activePeers(s).some((p) => p.connection === "connected"),
      "fixture-live-voice-not-connected",
    );
  }
  // A Live claims nothing: two started at the same moment both run.
  await Promise.all([
    click(f.owner, "Live gehen"),
    click(f.member, "Live gehen"),
  ]);
  const settled = await until(
    async () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
    (states) => states.every((s) => s.captures.some((c) => c.state === "live")),
    "concurrent-live-not-both-running",
  );
  await click(f.watcher, "Beitreten");
  const tiles = liveTiles(f.watcher.page);
  const shown = await until(
    () => tiles.count(),
    (count) => count === 2,
    "concurrent-live-tiles-missing",
  );
  return {
    simultaneousLives: 2,
    settled,
    tiles: shown,
    // Two Lives in the room: each one is measured by name (GEL-33).
    first: await progress(f.watcher, {
      ...options,
      publisher: `E2E ${f.owner.label}`,
    }),
    second: await progress(f.watcher, {
      ...options,
      publisher: `E2E ${f.member.label}`,
    }),
  };
}

// The retried Live's steps in ms after its click (GEL-460), from both pages'
// media RPC and consumer records: on every row, so that a slow retry that
// still passes says which step took its time.
export function retrySteps(clickedAt, retry, publisher, watcher) {
  const last = (flow, method) =>
    flow?.rpc?.filter((c) => c.method === method && c.kind === "s").at(-1);
  const after = (at) =>
    typeof at === "number" && typeof clickedAt === "number"
      ? at - clickedAt
      : null;
  const produce = last(publisher, "produce");
  const watch = last(watcher, "w");
  const ready = last(watcher, "consumerReady");
  const consumer = watcher?.consumers?.at(-1);
  return {
    produceSent: after(produce?.sentAt),
    produceAnswered: after(produce?.answeredAt),
    watchSent: after(watch?.sentAt),
    watchAnswered: after(watch?.answeredAt),
    consumerArrived: after(consumer?.arrivedAt),
    consumerReadySent: after(ready?.sentAt),
    consumerReadyAnswered: after(ready?.answeredAt),
    lastLayerReport: after(consumer?.layers?.at),
    layer: consumer?.layers
      ? { spatial: consumer.layers.spatial, temporal: consumer.layers.temporal }
      : null,
    firstFrame:
      typeof retry?.startedAt === "number"
        ? after(retry.startedAt + retry.firstFrameMs)
        : null,
  };
}
// Above this, a passing row keeps both pages' raw records too.
const SLOW_RETRY_MS = 1_500;

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
      () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
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
    // Packets arrive before the browser has attached the received track to
    // its audio element: wait until each side plays something. How many
    // sources and paths is the check's to judge, on a later sample: a second
    // path for the same source can come a moment after the first, so it
    // judges the sample in which the audio has moved on past this one.
    const attached = await until(
      () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
      (states) => states.every((s) => s.roomAudio.perSource.some((n) => n > 0)),
      "voice-only-duplex-playback-not-attached",
      options.budget,
    );
    const judged = await until(
      () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
      (states) =>
        states.every((s, i) => audioPackets(s) > audioPackets(attached[i])),
      "voice-only-duplex-audio-stopped-after-attach",
      options.budget,
    );
    for (const [i, s] of judged.entries()) {
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
        { before, after, attached, judged },
      );
      // Read until settled, as in progress() (GEL-465).
      await waitForPath(() => snapshot([f.owner, f.member][i]), options.relay);
    }
    return {
      before,
      after,
      attached,
      judged,
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
        await click(f.owner, "Kamera");
        const readPublisher = (remaining) =>
          nativeEvaluate(
            f.owner,
            sampleVideoSenders,
            { deadlineEpochMs: Date.now() + remaining },
            remaining,
          );
        const publisherBefore = await until(
          readPublisher,
          (s) => publisherEncoderProgress(s),
          "fixture-two-native-publisher-encoders-not-ready",
          20_000,
        );
        const publisherReady = await until(
          readPublisher,
          (s) => publisherEncoderProgress(s, publisherBefore),
          "fixture-two-native-publisher-encoders-not-progressing",
          5_000,
        );
        await click(f.member, "Beitreten");
        // Join/capture clicks precede async publication and source-Watch capability.
        // Require actual subscription controls; count=0 must not silently skip Watch.
        const sourceButtons = {};
        for (const kind of ["live"]) {
          const tiles = liveTiles(f.member.page);
          const start = tiles.getByRole("button", {
            name: "Zuschauen",
            exact: true,
          });
          await start.first().waitFor({ state: "visible", timeout: 20_000 });
          sourceButtons[kind] = await start.count();
          check(
            sourceButtons[kind] === 1,
            "fixture-source-watch-control-not-unique",
          );
          await start.first().click();
          await tiles
            .getByRole("button", { name: "Nicht mehr zuschauen", exact: true })
            .waitFor();
        }
        const held = await until(
          () => snapshot(f.member),
          (s) => s.heldVideoTracks === 2,
          "fixture-two-native-video-arrivals-not-held",
          20_000,
        );
        // Reverse authoritative Consumer announcements before the SDK consumes
        // them; the actual native peers and RTP remain unchanged.
        const kinds = await nativeEvaluate(f.member, () => {
          const state = window.__e2e;
          state.holdTracks = false;
          const pending = state.heldTracks.splice(0).reverse();
          const kinds = pending.map((item) => item.kind);
          for (const item of pending) item.deliver();
          return kinds;
        });
        return {
          publisherBefore,
          publisherReady,
          sourceButtons,
          held,
          consumerAnnouncementOrder: "reversed",
          deliveredKinds: kinds,
          live: await progress(f.member, options),
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
      // The camera is the other source; the Live is the one whose answer fails.
      await click(f.owner, "Kamera");
      const warmupStart = Date.now();
      const cameraFixtureWarmup = { deadlineMs: 20_000, status: "FAIL" };
      let camera, before;
      let retryClickedAt = null;
      try {
        const ready = await until(
          () => snapshot(f.member),
          (s) =>
            s.videos.some(
              (v) =>
                v.kind === "camera" &&
                [160, 320, 640].includes(v.width) &&
                v.height === (v.width * 9) / 16,
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
        await click(f.owner, "Live gehen");
        const failed = await until(
          () => snapshot(f.owner),
          (s) =>
            s.rejectedSdp === 1 &&
            s.displayCalls === before.displayCalls + 1 &&
            s.captures.filter((c) => c.kind === "display" && c.slot === 1)
              .length === 1 &&
            s.captures
              .filter((c) => c.kind === "display" && c.slot === 1)
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
        check(
          !afterCamera.last.videos.some(
            (v) => v.kind === "live" && v.width > 0,
          ),
          "rejected-sdp-ghost-live-display",
          { controlConfirmed: true, after: afterCamera.last },
        );
        // What the publisher sends per layer while the retry starts: evidence
        // only, so a sampler that cannot start fails nothing (GEL-460).
        await nativeEvaluate(f.owner, startSendTimeline, { kind: "s" }).catch(
          () => null,
        );
        retryClickedAt = Date.now();
        await click(f.owner, "Live gehen");
        // The retry is the second display capture, drawn green.
        const retry = await progress(f.member, {
          ...options,
          color: [30, 220, 30],
        });
        // Evidence only: a read that fails leaves nulls, never a failed row.
        const publisherLive = await nativeEvaluate(
          f.owner,
          samplePublicationFlow,
          { kind: "s" },
        ).catch(() => null);
        const subscriberLive = await nativeEvaluate(
          f.member,
          samplePublicationFlow,
          { kind: "s" },
        ).catch(() => null);
        return {
          retrySteps: retrySteps(
            retryClickedAt,
            retry,
            publisherLive,
            subscriberLive,
          ),
          ...(retry.firstFrameMs > SLOW_RETRY_MS
            ? {
                publisherLive,
                subscriberLive,
                sendTimeline: await nativeEvaluate(f.owner, sendTimeline).catch(
                  () => null,
                ),
              }
            : {}),
          injection:
            "one real native setRemoteDescription rejection of malformed answer",
          cameraFixtureWarmup,
          faultExercised: true,
          sdpRejectionErrors: failed.rejectedSdpErrors,
          before,
          failed,
          camera,
          afterCamera,
          retry,
        };
      } catch (error) {
        const observed = await snapshot(f.owner).catch(() => null);
        const diagnostics = {
          cameraFixtureWarmup,
          // With the RPC and consumer times below, where a slow retry spent
          // its time (GEL-460).
          retryClickedAt,
          faultExercised: observed?.rejectedSdp > 0,
          sdpRejectionErrors: observed?.rejectedSdpErrors ?? [],
          publisherLive: await nativeEvaluate(f.owner, samplePublicationFlow, {
            kind: "s",
          }),
          subscriberLive: await nativeEvaluate(
            f.member,
            samplePublicationFlow,
            { kind: "s" },
          ),
        };
        if (retryClickedAt !== null) {
          diagnostics.retrySteps = retrySteps(
            retryClickedAt,
            null,
            diagnostics.publisherLive,
            diagnostics.subscriberLive,
          );
          diagnostics.sendTimeline = await nativeEvaluate(
            f.owner,
            sendTimeline,
          ).catch(() => null);
        }
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
  // One call, room A: the owner is live, the member talks, and the watcher
  // joins with playback refused. The visible retry starts every audio path
  // of the room; mute, deafen and the call volume then act on all of them.
  await h.run(
    "session-audio-mute-deafen-volume-playback-retry",
    ["08a", "08b"],
    async () => {
      await reset(f);
      await click(f.owner, "Beitreten");
      await click(f.owner, "Live gehen");
      await click(f.member, "Beitreten");
      await until(
        () => snapshot(f.member),
        (s) => audioPackets(s) > 0,
        "fixture-bidirectional-audio-not-received",
      );
      const audioPlays = (s) =>
        s.playback.filter((p) => p.kind === "audio" && p.audioTracks > 0);
      // The owner's and the member's microphone.
      const paths = 2;
      let stage = "watcher-voice-join";
      try {
        await nativeEvaluate(f.watcher, () => {
          window.__e2e.rejectPlayback = true;
        });
        await click(f.watcher, "Beitreten");
        // First the room's paths, under a name of their own: a participant
        // left over from another scenario shows up here, not as a refusal.
        await until(
          () => snapshot(f.watcher),
          (s) => audioPlays(s).length === paths,
          "fixture-room-audio-paths-not-two",
        );
        const blocked = await until(
          () => snapshot(f.watcher),
          (s) => s.playRejected > 0,
          "fixture-room-audio-playback-not-refused",
        );
        const retry = f.watcher.page.getByRole("button", {
          name: "Ton starten",
          exact: true,
        });
        await until(
          () => retry.count(),
          (count) => count > 0,
          "audio-playback-failure-no-visible-retry",
          5_000,
        );
        // The fault lifts with the real click on the retry, not before. A
        // refused element can still report playing (autoplay), so the retry
        // counts by the play() calls it gets through, one per path, and the
        // notice going away.
        await nativeEvaluate(f.watcher, armPlaybackRetry);
        stage = "visible-audio-retry";
        const before = (await snapshot(f.watcher)).audioPlayStarted;
        await retry.first().click();
        const playing = await until(
          async () => ({
            ...(await snapshot(f.watcher)),
            retryShown: await retry.count(),
          }),
          (s) =>
            s.audioPlayStarted - before >= paths &&
            s.retryShown === 0 &&
            audioPlays(s).length === paths &&
            audioPlays(s).every((p) => !p.paused && !p.muted) &&
            !s.duplicateAudioPlaybackTracks,
          "audio-click-retry-did-not-play-every-path",
        );
        stage = "watch-live";
        await progress(f.watcher, options);
        await nativeEvaluate(
          f.watcher,
          (publishers) => {
            window.__e2e.expectedAudioPublishers = publishers;
            window.__e2e.expectedLivePublisher = publishers[0];
          },
          [f.owner.id, f.member.id],
        );
        const selected = await until(
          () => snapshot(f.watcher),
          (s) =>
            s.roomAudio.perSource.length === paths &&
            s.roomAudio.perSource.every((n) => n === 1),
          "fixture-each-room-audio-source-not-received",
        );
        check(
          selected.roomAudio.foreign === 0 &&
            selected.watchVideoSources.selectedLive === 1 &&
            selected.watchVideoSources.foreign === 0 &&
            selected.duplicateAudioPlaybackTracks === 0 &&
            // A Live is watched inside the call, which takes the microphone
            // once (since 2026-10-22); watching takes no second capture.
            selected.micCalls === 1,
          "watch-foreign-source-or-duplicate-audio",
          { selected },
        );
        const microphoneControl = await until(
          async () => {
            const observed = await snapshot(f.watcher);
            const control = await nativeEvaluate(
              f.watcher,
              async function sample({ deadlineEpochMs }) {
                const state = window.__e2e;
                const candidates = state.peers
                  .filter((peer) => peer.connectionState === "connected")
                  .flatMap((peer) =>
                    peer.getSenders().map((sender) => ({ peer, sender })),
                  )
                  .filter(
                    ({ sender }) =>
                      sender.track?.kind === "audio" &&
                      sender.track.readyState === "live",
                  );
                const current = candidates.length === 1 ? candidates[0] : null;
                state.sessionAudioMicrophone = current
                  ? { ...current, track: current.sender.track }
                  : null;
                let outboundAudioPackets = null;
                if (current && typeof current.sender.getStats === "function") {
                  const remaining = deadlineEpochMs - Date.now();
                  if (remaining <= 0)
                    throw new Error("E2E_NATIVE_STATS_DEADLINE");
                  let timer;
                  let report;
                  try {
                    report = await Promise.race([
                      current.sender.getStats(),
                      new Promise((_, reject) => {
                        timer = setTimeout(
                          () => reject(new Error("E2E_NATIVE_STATS_DEADLINE")),
                          Math.max(
                            0,
                            remaining -
                              Math.min(100, Math.max(1, remaining / 10)),
                          ),
                        );
                      }),
                    ]);
                  } finally {
                    clearTimeout(timer);
                  }
                  const audioReports = [...report.values()].filter(
                    (entry) =>
                      entry.type === "outbound-rtp" &&
                      (entry.kind ?? entry.mediaType) === "audio",
                  );
                  if (
                    audioReports.length > 0 &&
                    audioReports.every(
                      (entry) =>
                        Number.isInteger(entry.packetsSent) &&
                        entry.packetsSent >= 0,
                    )
                  )
                    outboundAudioPackets = audioReports.reduce(
                      (sum, entry) => sum + entry.packetsSent,
                      0,
                    );
                }
                return {
                  senderCount: candidates.length,
                  peerIndex: current ? state.peers.indexOf(current.peer) : null,
                  enabled: current ? current.sender.track.enabled : null,
                  outboundAudioPackets,
                };
              },
            );
            return {
              ...control,
              outboundAudioPackets:
                observed.peers[control.peerIndex]?.connection === "connected"
                  ? control.outboundAudioPackets
                  : null,
            };
          },
          (s) =>
            s.senderCount === 1 &&
            s.enabled === true &&
            s.outboundAudioPackets > 0,
          "fixture-session-positive-microphone-not-ready",
          5_000,
        );
        const microphoneSnapshot = async () => {
          const observed = await snapshot(f.watcher);
          const sessionMicrophone = await nativeEvaluate(f.watcher, () => {
            const state = window.__e2e,
              expected = state.sessionAudioMicrophone;
            const candidates = state.peers
              .filter((peer) => peer.connectionState === "connected")
              .flatMap((peer) =>
                peer.getSenders().map((sender) => ({ peer, sender })),
              )
              .filter(
                ({ sender }) =>
                  sender.track?.kind === "audio" &&
                  sender.track.readyState === "live",
              );
            const current = candidates[0];
            return {
              senderCount: candidates.length,
              expectedRetained:
                candidates.length === 1 &&
                !!expected &&
                current.peer === expected.peer &&
                current.sender === expected.sender &&
                current.sender.track === expected.track,
              enabled:
                candidates.length === 1 ? current.sender.track.enabled : null,
            };
          });
          return { ...observed, sessionMicrophone };
        };
        stage = "session-mic-mute";
        await click(f.watcher, "Stummschalten");
        const muted = await until(
          microphoneSnapshot,
          (s) =>
            audioSenders(s).length === 1 &&
            audioSenders(s).every((t) => !t.enabled) &&
            s.sessionMicrophone.expectedRetained &&
            s.sessionMicrophone.enabled === false,
          "session-mute-did-not-disable-mic",
        );
        stage = "session-deafen";
        await click(f.watcher, "Stummschalten");
        await click(f.watcher, "Taubschalten");
        const deafened = await until(
          microphoneSnapshot,
          (s) =>
            audioPlays(s).length === paths &&
            audioPlays(s).every((p) => p.muted) &&
            audioSenders(s).length === 1 &&
            audioSenders(s).every((t) => !t.enabled) &&
            s.sessionMicrophone.expectedRetained &&
            s.sessionMicrophone.enabled === false,
          "session-deafen-did-not-mute-every-path",
        );
        stage = "session-undeafen";
        await click(f.watcher, "Taubschalten");
        const hearing = await until(
          microphoneSnapshot,
          (s) =>
            audioPlays(s).length === paths &&
            audioPlays(s).every((p) => !p.muted && !p.paused) &&
            audioSenders(s).length === 1 &&
            audioSenders(s).every((t) => t.enabled) &&
            s.sessionMicrophone.expectedRetained &&
            s.sessionMicrophone.enabled === true,
          "session-undeafen-did-not-restore-every-path",
        );
        stage = "session-volume";
        // While a Live plays in the call, the call bar's "Lautstärke" holds
        // the call volume ("Gespräche") beside the Live's own.
        await f.watcher.page
          .getByRole("region", { name: "Aktive Medien", exact: true })
          .getByRole("button", { name: "Lautstärke", exact: true })
          .click();
        const volume = f.watcher.page
          .getByRole("dialog", { name: "Lautstärke", exact: true })
          .getByRole("slider", { name: "Gespräche", exact: true });
        await volume.press("Home");
        for (let i = 0; i < 35; i++) await volume.press("ArrowRight");
        const lower = await until(
          () => snapshot(f.watcher),
          (s) =>
            audioPlays(s).length === paths &&
            audioPlays(s).every((p) => Math.abs(p.volume - 0.35) < 0.001),
          "session-volume-not-applied-to-every-path",
        );
        return {
          blocked,
          playing,
          selected,
          microphoneControl,
          muted,
          deafened,
          hearing,
          lower,
          contract:
            "every audio path of the call exactly once, started by the visible retry; only the selected Live's video",
          limitation:
            "native transport/playback properties only; human audible two-device quality remains manual",
        };
      } catch (error) {
        if (error instanceof CheckFailure) throw error;
        throw new CheckFailure("session-audio-interface-error", {
          stage,
          last: await snapshot(f.watcher).catch(() => null),
        });
      } finally {
        await restoreSessionPlayback(f.watcher);
      }
    },
  );
}
