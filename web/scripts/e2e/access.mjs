/* global window, document, MediaStream, URL, fetch, setTimeout, clearTimeout */
import {
  nativeEvaluate,
  NativeInterfaceFailure,
  deadlineProbe,
} from "./native-evaluate.mjs";
import {
  api as harnessApi,
  check,
  click,
  navigate,
  observe,
  until,
} from "./harness.mjs";
import { progress } from "./media.mjs";
import { attemptAll } from "./teardown.mjs";
const api = (actor, path, method, body) =>
  harnessApi(actor, path, method, body, 10_000);
async function restoreOwnerAccount(actor, base) {
  const current = await api(actor, "/auth/session");
  if (current.body.user?.id === actor.id) return;
  if (current.body.user) await api(actor, "/auth/logout", "POST");
  await navigate(actor, "/login", base);
  await actor.page.getByLabel("E-Mail-Adresse").fill(actor.email);
  await actor.page.getByLabel("Passwort", { exact: true }).fill(actor.password);
  await click(actor, "Anmelden");
  await actor.page.waitForURL((u) => !u.pathname.includes("login"));
  check(
    (await api(actor, "/auth/session")).body.user?.id === actor.id,
    "fixture-account-restoration-failed",
  );
}

// A second native socket intentionally ignores UI auth cleanup. Store only
// counts/known protocol outcomes; private event bodies never leave the browser.
async function maliciousGateway(actor, serverId, channelId) {
  await nativeEvaluate(
    actor,
    async ({ serverId, channelId }) => {
      const state = window.__e2e;
      const url = new URL("/ws", window.location.href);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      const ws = new state.NativeSocket(url);
      const probe = (state.rawGateway = { ws, events: 0, subscribed: false });
      ws.addEventListener("message", (event) => {
        const frame = JSON.parse(event.data);
        if (frame.op === "h") ws.send('{"op":"h"}');
        if (frame.op === "ok" && frame.c === channelId) probe.subscribed = true;
        if (frame.op === "e" && frame.c === channelId) probe.events++;
      });
      await new Promise((resolve, reject) => {
        ws.addEventListener("open", resolve, { once: true });
        ws.addEventListener("error", reject, { once: true });
      });
      ws.send(JSON.stringify({ op: "s", s: serverId, c: channelId }));
    },
    { serverId, channelId },
  );
  await until(
    () =>
      nativeEvaluate(actor, () => ({
        subscribed: window.__e2e.rawGateway.subscribed,
      })),
    (s) => s.subscribed,
    "malicious-gateway-not-subscribed",
  );
}
async function rawSample(actor) {
  return nativeEvaluate(actor, () => ({
    state: window.__e2e.rawGateway.ws.readyState,
    events: window.__e2e.rawGateway.events,
  }));
}
async function heldTicket(actor, channelId) {
  // The ticket stays in browser memory, never in report or command output.
  const result = await nativeEvaluate(
    actor,
    async (channelId) => {
      const session = await fetch("/api/auth/session").then((r) => r.json());
      const r = await fetch(`/api/channels/${channelId}/media-ticket`, {
        method: "POST",
        headers: { "X-CSRF-Token": session.csrf_token },
      });
      if (r.status === 200) window.__e2e.heldTicket = await r.json();
      return { status: r.status };
    },
    channelId,
  );
  check(result.status === 200, "pre-revocation-ticket-not-issued", result);
}
async function useHeldTicket(actor) {
  await nativeEvaluate(actor, async () => {
    const state = window.__e2e,
      ticket = state.heldTicket;
    const url = new URL(ticket.media_path, window.location.href);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    const ws = new state.NativeSocket(url);
    const probe = (state.rawTicket = { ws, accepted: false, denied: false });
    ws.addEventListener("message", (event) => {
      const f = JSON.parse(event.data);
      if (f.op === "result" && f.id === 1) probe.accepted = f.data?.v === 4;
      if (f.op === "err")
        probe.denied = ["unauthorized", "gone", "forbidden"].includes(f.e);
    });
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    ws.send(JSON.stringify({ op: "j", id: 1, v: 4, tk: ticket.ticket }));
  });
  const outcome = await until(
    () =>
      nativeEvaluate(actor, () => ({
        accepted: window.__e2e.rawTicket.accepted,
        denied: window.__e2e.rawTicket.denied,
        state: window.__e2e.rawTicket.ws.readyState,
      })),
    (s) => s.accepted || s.denied || s.state === 3,
    "held-ticket-no-result",
    5_000,
  );
  check(
    !outcome.accepted && (outcome.denied || outcome.state === 3),
    "revoked-unconsumed-ticket-accepted",
    outcome,
  );
  return outcome;
}
export async function holdActiveMedia(
  actor,
  { renderer = false, suppressClientCleanupRequests = false } = {},
) {
  await nativeEvaluate(
    actor,
    async ({ renderer, suppressClientCleanupRequests }) => {
      const state = window.__e2e;
      state.heldCounterMode = renderer ? "receiver-renderer" : "rtp";
      state.heldRenderers = [];
      state.heldExpectedRenderers = [];
      state.heldProbators = [];
      state.heldFrameCounterKeys = null;
      state.heldSuppressedClientCleanupRequests = {
        l: 0,
        closeTransport: 0,
        consumerFailed: 0,
      };
      state.heldNativeNegotiationRequests = {
        setRemoteDescription: 0,
        setLocalDescription: 0,
      };
      state.heldNativeNegotiationWaits = [];
      state.heldPeers = state.peers.filter(
        (p) => p.connectionState === "connected",
      );
      state.heldSockets = state.sockets.filter(
        (s) => s.plane === "media" && s.ws.readyState === 1,
      );
      state.restoreClose = [];
      state.holdMediaCleanup = true;
      state.restoreClose.push(() => {
        state.holdMediaCleanup = false;
      });
      // Ignore client-side teardown to test the SFU boundary independently.
      for (const pc of state.heldPeers) {
        if (renderer) {
          // SDK Consumer.close may start native SDP teardown before its
          // Transport.close reaches pc.close. Keep the original negotiation
          // alive without inventing a native success or allowing local teardown
          // to masquerade as stopped server media. Release rejects every wait.
          for (const method of [
            "setRemoteDescription",
            "setLocalDescription",
          ]) {
            const original = pc[method];
            pc[method] = () => {
              const counts = state.heldNativeNegotiationRequests;
              if (state.heldNativeNegotiationWaits.length >= 64)
                return Promise.reject(
                  new Error("E2E_HELD_NATIVE_NEGOTIATION_OVERFLOW"),
                );
              counts[method]++;
              const pending = new Promise((_resolve, reject) => {
                state.heldNativeNegotiationWaits.push({ reject });
              });
              pending.catch(() => {});
              return pending;
            };
            state.restoreClose.push(() => {
              pc[method] = original;
            });
          }
        }
        state.restoreClose.push(() =>
          window.RTCPeerConnection.prototype.close.call(pc),
        );
        pc.close = () => {};
        // Transport.close also closes SDK Consumers; keep the exact native
        // receiver tracks alive so client teardown cannot prove server revoke.
        for (const receiver of pc.getReceivers?.() ?? []) {
          const track = receiver.track;
          const stop = track.stop.bind(track);
          track.stop = () => {};
          state.restoreClose.push(() => {
            track.stop = stop;
            stop();
          });
        }
      }
      for (const { ws } of state.heldSockets) {
        if (suppressClientCleanupRequests) {
          const send = ws.send;
          state.restoreClose.push(() => {
            ws.send = send;
          });
          // External ACL revocation must stop the held peer itself. In v4,
          // client Leave retires the peer while retaining an unjoined socket;
          // allowing it would prove client cleanup instead of server authority.
          // Do not acknowledge suppressed RPCs or affect Watch-Off controls.
          ws.send = function (data) {
            let frame;
            try {
              frame = typeof data === "string" ? JSON.parse(data) : null;
            } catch {
              /* Unknown/binary traffic keeps its original native behavior. */
            }
            if (
              frame &&
              ["l", "closeTransport", "consumerFailed"].includes(frame.op) &&
              Number.isInteger(frame.id) &&
              frame.id > 0 &&
              frame.id <= 0xffffffff
            ) {
              const counts = state.heldSuppressedClientCleanupRequests;
              if (counts[frame.op] >= 64)
                throw new Error("E2E_HELD_CLIENT_CLEANUP_OVERFLOW");
              counts[frame.op]++;
              return;
            }
            return send.call(this, data);
          };
        }
        state.restoreClose.push(() =>
          state.NativeSocket.prototype.close.call(ws),
        );
        ws.close = () => {};
      }
      state.restoreClose.push(() => {
        for (const wait of state.heldNativeNegotiationWaits.splice(0))
          wait.reject(new Error("E2E_HELD_NATIVE_NEGOTIATION_RELEASED"));
      });
      if (renderer) {
        const renderSources = [];
        const sourceKeys = new Set();
        const sourceSsrcs = new Set();
        let probators = 0;
        for (const pc of state.heldPeers) {
          for (const receiver of pc.getReceivers()) {
            const track = receiver.track;
            if (track.kind !== "video" || track.readyState !== "live") continue;
            const sources = (state.incomingTracks ?? []).filter(
              (source) =>
                source.pc === pc &&
                source.receiver === receiver &&
                source.track === track,
            );
            const transceivers = (pc.getTransceivers?.() ?? []).filter(
              (transceiver) => transceiver.receiver === receiver,
            );
            if (sources.length === 0) {
              // Pinned mediasoup-client creates one non-Consumer video probator
              // with MID/trackId "probator", sole codec PT 127 and SSRC 1234.
              // A native RTP report may not exist before probation packets ran.
              // Prove its role using public native objects; absent counters stay
              // unavailable. Never exempt a source-bound or unknown receiver.
              const codecs = receiver.getParameters?.().codecs;
              if (
                track.id !== "probator" ||
                ++probators > 1 ||
                transceivers.length !== 1 ||
                transceivers[0].mid !== "probator" ||
                !Array.isArray(codecs) ||
                codecs.length !== 1 ||
                codecs[0].payloadType !== 127 ||
                typeof codecs[0].mimeType !== "string" ||
                !/^video\/(VP8|VP9|H264|H265|AV1)$/i.test(codecs[0].mimeType) ||
                (state.incomingTracks ?? []).some(
                  (source) =>
                    source.receiver === receiver || source.track === track,
                )
              )
                throw new Error("E2E_HELD_UNBOUND_VIDEO_RECEIVER");
              const rows = [...(await receiver.getStats()).values()].filter(
                (row) => row.type === "inbound-rtp",
              );
              const currentTransceivers = pc.getTransceivers();
              const currentCodecs = receiver.getParameters?.().codecs;
              if (
                receiver.track !== track ||
                track.id !== "probator" ||
                !pc.getReceivers().includes(receiver) ||
                currentTransceivers.filter(
                  (transceiver) => transceiver.receiver === receiver,
                ).length !== 1 ||
                !currentTransceivers.includes(transceivers[0]) ||
                transceivers[0].receiver !== receiver ||
                transceivers[0].mid !== "probator" ||
                !Array.isArray(currentCodecs) ||
                currentCodecs.length !== 1 ||
                currentCodecs[0].payloadType !== 127 ||
                currentCodecs[0].mimeType !== codecs[0].mimeType ||
                (state.incomingTracks ?? []).some(
                  (source) =>
                    source.receiver === receiver || source.track === track,
                ) ||
                (rows.length !== 0 &&
                  (rows.length !== 1 ||
                    (rows[0].kind ?? rows[0].mediaType) !== "video" ||
                    rows[0].ssrc !== 1234 ||
                    !Number.isFinite(rows[0].packetsReceived) ||
                    rows[0].packetsReceived < 0 ||
                    !Number.isFinite(rows[0].bytesReceived) ||
                    rows[0].bytesReceived < 0 ||
                    rows[0].framesDecoded !== 0))
              )
                throw new Error("E2E_HELD_PROBATOR_NATIVE_REPORT_UNAVAILABLE");
              state.heldProbators.push({
                mid: transceivers[0].mid,
                codecPayload: codecs[0].payloadType,
                mime: codecs[0].mimeType,
                rtpStatsAvailable: rows.length === 1,
              });
              continue;
            }
            const source = sources[0];
            const identity = Object.fromEntries(
              [
                "publisher",
                "sourceKind",
                "consumerId",
                "producerId",
                "epoch",
                "generation",
              ].map((key) => [key, source[key]]),
            );
            const encodings = source.rtpParameters?.encodings;
            const sourceKey = `${source.consumerId}:${source.generation}`;
            if (
              sources.length !== 1 ||
              transceivers.length !== 1 ||
              typeof transceivers[0].mid !== "string" ||
              transceivers[0].mid.length === 0 ||
              !["recvonly", "sendrecv"].includes(
                transceivers[0].currentDirection,
              ) ||
              track.id === "probator" ||
              transceivers.some(
                (transceiver) => transceiver.mid === "probator",
              ) ||
              !["v", "s", "l"].includes(identity.sourceKind) ||
              !Object.values(identity).every(
                (value) =>
                  typeof value === "string" &&
                  value.length > 0 &&
                  value.length <= 128,
              ) ||
              !Array.isArray(encodings) ||
              encodings.length !== 1 ||
              !Number.isInteger(encodings[0].ssrc) ||
              encodings[0].ssrc <= 0 ||
              encodings[0].ssrc > 0xffffffff ||
              encodings[0].ssrc === 1234 ||
              sourceKeys.has(sourceKey) ||
              sourceSsrcs.has(encodings[0].ssrc)
            )
              throw new Error("E2E_HELD_CONSUMER_IDENTITY_UNAVAILABLE");
            sourceKeys.add(sourceKey);
            sourceSsrcs.add(encodings[0].ssrc);
            renderSources.push({
              pc,
              receiver,
              track,
              consumerSource: source,
              consumerIdentity: identity,
              consumerSsrc: encodings[0].ssrc,
              transceiver: transceivers[0],
              transceiverMid: transceivers[0].mid,
              currentDirection: transceivers[0].currentDirection,
            });
          }
        }
        for (const bound of renderSources) {
          const {
            pc,
            receiver,
            track,
            consumerSource,
            consumerIdentity,
            consumerSsrc,
            transceiver,
            transceiverMid,
            currentDirection,
          } = bound;
          const video = document.createElement("video");
          video.muted = true;
          video.playsInline = true;
          video.style.cssText =
            "position:fixed;left:-10000px;width:16px;height:16px";
          video.srcObject = new MediaStream([track]);
          const callbackCounter =
            typeof video.requestVideoFrameCallback === "function";
          const item = {
            receiver,
            track,
            pc,
            consumerSource,
            video,
            frames: 0,
            lastObservedFrames: 0,
            callback: null,
            callbackFunction: null,
            counterFailed: false,
            disposed: false,
            source: callbackCounter
              ? "native-video-frame-callback"
              : "native-playback-quality",
          };
          state.heldRenderers.push(item);
          const expected = {
            renderer: item,
            source: item.source,
            video,
            receiver,
            track,
            pc,
            consumerSource,
            consumerIdentity,
            consumerSsrc,
            transceiver,
            transceiverMid,
            currentDirection,
            callbackFunction: null,
          };
          state.heldExpectedRenderers.push(expected);
          document.body.append(video);
          if (callbackCounter) {
            const onFrame = (_now, metadata) => {
              if (item.disposed || item.counterFailed) return;
              if (
                !Number.isSafeInteger(metadata.presentedFrames) ||
                metadata.presentedFrames < item.frames
              ) {
                item.counterFailed = true;
                return;
              }
              item.frames = metadata.presentedFrames;
              try {
                item.callback = video.requestVideoFrameCallback(onFrame);
              } catch {
                item.counterFailed = true;
              }
            };
            item.callbackFunction = onFrame;
            expected.callbackFunction = onFrame;
            item.callback = video.requestVideoFrameCallback(onFrame);
          }
          await video.play();
        }
      }
    },
    { renderer, suppressClientCleanupRequests },
  );
  const baseline = await heldMedia(actor);
  if (renderer) {
    const positive = await until(
      () => heldMedia(actor),
      (s) => s.counters.every((counter) => counter.frames > 0),
      "fixture-held-renderer-positive-frames-missing",
      5_000,
    );
    return until(
      () => heldMedia(actor),
      (s) =>
        s.counters.every(
          (counter, i) => counter.frames > positive.counters[i].frames + 1,
        ),
      "fixture-held-renderer-positive-progress-missing",
      5_000,
    );
  }
  check(
    baseline.frames > 0,
    "fixture-held-positive-decoded-counter-missing",
    baseline,
  );
  return baseline;
}
export async function heldMedia(actor) {
  const measurement = await nativeEvaluate(
    actor,
    async function sample({ deadlineEpochMs }) {
      const state = window.__e2e;
      if (state.heldCounterMode === "receiver-renderer") {
        const counters = state.heldRenderers.map((item, i) => {
          const expected = state.heldExpectedRenderers[i];
          const binding = {
            sameRenderer:
              expected?.renderer === item && expected?.video === item.video,
            sameReceiverTrack:
              expected?.receiver === item.receiver &&
              expected?.track === item.track &&
              item.receiver.track === item.track,
            sameNativePeer:
              expected?.pc === item.pc &&
              item.pc.getReceivers().includes(item.receiver),
            sameNativeTransceiver:
              item.pc.getTransceivers().includes(expected?.transceiver) &&
              item.pc
                .getTransceivers()
                .filter((transceiver) => transceiver.receiver === item.receiver)
                .length === 1 &&
              expected.transceiver.receiver === item.receiver &&
              expected.transceiver.mid === expected.transceiverMid &&
              ["recvonly", "sendrecv"].includes(expected.currentDirection) &&
              expected.transceiver.currentDirection ===
                expected.currentDirection,
            sameConsumerIdentity:
              expected?.consumerSource === item.consumerSource &&
              state.incomingTracks.includes(item.consumerSource) &&
              item.consumerSource.pc === item.pc &&
              item.consumerSource.receiver === item.receiver &&
              item.consumerSource.track === item.track &&
              Object.entries(expected.consumerIdentity).every(
                ([key, value]) => item.consumerSource[key] === value,
              ) &&
              item.consumerSource.rtpParameters?.encodings?.length === 1 &&
              item.consumerSource.rtpParameters.encodings[0].ssrc ===
                expected.consumerSsrc,
            liveTrack: item.track.readyState === "live",
            enabledTrack: item.track.enabled === true,
            sameCallback: expected?.callbackFunction === item.callbackFunction,
          };
          const sameSource =
            !item.disposed &&
            !item.counterFailed &&
            !item.video.error &&
            !item.video.paused &&
            Object.values(binding).every(Boolean) &&
            item.video.srcObject?.getVideoTracks().length === 1 &&
            item.video.srcObject.getVideoTracks()[0] === item.track;
          const frames =
            item.source === "native-video-frame-callback"
              ? item.frames
              : item.video.getVideoPlaybackQuality?.().totalVideoFrames;
          const available =
            sameSource &&
            Number.isSafeInteger(frames) &&
            frames >= item.lastObservedFrames;
          if (available) item.lastObservedFrames = frames;
          return {
            renderer: i,
            receiverTrack: i,
            callback: item.source === "native-video-frame-callback" ? i : null,
            binding,
            source: item.source,
            frames: available ? frames : null,
          };
        });
        const complete =
          counters.length > 0 &&
          state.heldExpectedRenderers.length === counters.length &&
          state.heldExpectedRenderers.every(
            (item, i) =>
              item.renderer === state.heldRenderers[i] &&
              item.source === state.heldRenderers[i].source,
          ) &&
          counters.every((counter) => counter.frames !== null);
        return {
          frames: complete
            ? counters.reduce((sum, counter) => sum + counter.frames, 0)
            : null,
          frameCountersAvailable: complete,
          counterMode: "receiver-renderer",
          counters,
          expectedFrameCounters: state.heldExpectedRenderers.length,
          // These are setup role observations, never manufactured RTP counters.
          heldProbators: state.heldProbators ?? [],
          excludedSdkProbators: state.heldProbators?.length ?? 0,
          probatorRtpStatsUnavailable: (state.heldProbators ?? []).filter(
            (probator) => !probator.rtpStatsAvailable,
          ).length,
          suppressedClientCleanupRequests:
            state.heldSuppressedClientCleanupRequests,
          heldNativeNegotiationRequests: state.heldNativeNegotiationRequests,
          openSockets: state.heldSockets.filter((s) => s.ws.readyState === 1)
            .length,
        };
      }
      let frames = 0,
        videoRtpEntries = 0;
      const availableKeys = [];
      for (const [peerIndex, pc] of state.heldPeers.entries()) {
        window.__e2e.samplePhase = "native-getStats";
        let timer;
        let stats;
        try {
          const remaining = deadlineEpochMs - Date.now();
          if (remaining <= 0) throw new Error("E2E_NATIVE_STATS_DEADLINE");
          stats = await Promise.race([
            pc.getStats(),
            new Promise((_, reject) => {
              timer = setTimeout(
                () => reject(new Error("E2E_NATIVE_STATS_DEADLINE")),
                Math.max(
                  0,
                  remaining - Math.min(100, Math.max(1, remaining / 10)),
                ),
              );
            }),
          ]);
        } finally {
          clearTimeout(timer);
        }
        window.__e2e.samplePhase = "native-stats-resolved";
        for (const [key, r] of stats.entries())
          if (r.type === "inbound-rtp" && (r.kind ?? r.mediaType) === "video") {
            videoRtpEntries++;
            if (Number.isInteger(r.framesDecoded) && r.framesDecoded >= 0) {
              availableKeys.push(`${peerIndex}:${key}`);
              frames += r.framesDecoded;
            }
          }
      }
      // Counter IDs stay in browser memory. A disappearing/replaced report cannot
      // reduce the observed sum and then masquerade as stagnant decoded media.
      const complete =
        videoRtpEntries > 0 && availableKeys.length === videoRtpEntries;
      if (complete && !state.heldFrameCounterKeys)
        state.heldFrameCounterKeys = availableKeys;
      const expectedKeys = state.heldFrameCounterKeys ?? [];
      const frameCountersAvailable =
        complete &&
        expectedKeys.length === availableKeys.length &&
        expectedKeys.every((key) => availableKeys.includes(key));
      return {
        frames: frameCountersAvailable ? frames : null,
        frameCountersAvailable,
        videoRtpEntries,
        availableFrameCounters: availableKeys.length,
        expectedFrameCounters: expectedKeys.length,
        suppressedClientCleanupRequests:
          state.heldSuppressedClientCleanupRequests,
        openSockets: window.__e2e.heldSockets.filter(
          (s) => s.ws.readyState === 1,
        ).length,
      };
    },
  );
  check(
    measurement.frameCountersAvailable,
    "fixture-held-video-counters-unavailable",
    measurement,
  );
  return measurement;
}
export async function releaseHeld(actor) {
  if (actor.nativeEvaluationUnusable || actor.page.isClosed?.()) return;
  await nativeEvaluate(actor, () => {
    const state = window.__e2e;
    // Dispose every renderer before restoring peers, even if one cleanup fails.
    let failed = false;
    for (const item of state.heldRenderers ?? []) {
      item.disposed = true;
      for (const cleanup of [
        () => {
          if (item.callback !== null)
            item.video.cancelVideoFrameCallback?.(item.callback);
        },
        () => item.video.pause(),
        () => {
          item.video.srcObject = null;
        },
        () => item.video.remove(),
      ]) {
        try {
          cleanup();
        } catch {
          failed = true;
        }
      }
    }
    for (const restore of state.restoreClose ?? []) {
      try {
        restore();
      } catch {
        failed = true;
      }
    }
    state.rawGateway?.ws.close();
    state.rawTicket?.ws.close();
    if (failed) throw new Error("held-renderer-cleanup-failed");
  }).catch((error) => {
    if (error instanceof NativeInterfaceFailure) throw error;
    if (!actor.page.isClosed?.()) throw error;
  });
}
export async function closeAccessActors(
  actors,
  restoreActors = [],
  budgetMs = 5_000,
) {
  const failures = await attemptAll([
    ...restoreActors.map((actor, i) => [
      `held-restore-${i}`,
      () => releaseHeld(actor),
    ]),
    ...actors.map((actor, i) => [
      `context-close-${i}`,
      () => deadlineProbe(() => actor.context.close(), Date.now() + budgetMs),
    ]),
  ]);
  if (failures.length)
    throw new NativeInterfaceFailure({
      stage: "access-restore-or-context-close",
      failedSteps: failures,
      nativeDataAvailable: false,
    });
}
async function closeIndependentSession(page, context) {
  const failedSteps = await attemptAll([
    ...(page
      ? [
          [
            "independent-session-logout",
            async () => {
              const logout = await api({ page }, "/auth/logout", "POST");
              check(
                logout.status === 200,
                "independent-session-cleanup-logout-failed",
                { status: logout.status },
              );
            },
          ],
        ]
      : []),
    ["independent-context-close", () => closeAccessActors([{ context }])],
  ]);
  if (failedSteps.length)
    throw new NativeInterfaceFailure({
      stage: "independent-session-owned-cleanup",
      failedSteps,
      nativeDataAvailable: false,
    });
}
export async function accessScenarios(h, f) {
  h.setIsolation(() => restoreOwnerAccount(f.owner, f.base));
  const chat = f.textPath.split("/").at(-1),
    voice = f.voicePath.split("/").at(-1);
  for (const mode of ["leave", "kick", "ban", "logout"]) {
    await h.run(
      `${mode}-revokes-existing-gateway-active-sfu-and-held-ticket`,
      ["03a", "03b"],
      async () => {
        const victim = await h.actor(`RevokedWatch${mode}`);
        try {
          await f.join(victim);
          await navigate(f.owner, f.voicePath, f.base);
          await click(f.owner, "Beitreten");
          await click(f.owner, "Go Live");
          await click(victim, "Zuschauen");
          await progress(victim, {
            relay: h.relay,
            budget: h.relay ? 10_000 : 5_000,
          });
          await maliciousGateway(victim, f.serverId, chat);
          const gatewayControlBefore = await rawSample(victim);
          const controlMessage = await api(
            f.owner,
            `/channels/${chat}/messages`,
            "POST",
            { content: "E2E pre-revocation control event", attachment_ids: [] },
          );
          check(
            [200, 201].includes(controlMessage.status),
            "control-message-create-failed",
            { status: controlMessage.status },
          );
          const gatewayControl = await until(
            () => rawSample(victim),
            (s) => s.events > gatewayControlBefore.events,
            "pre-revocation-gateway-control-failed",
          );
          await heldTicket(victim, voice);
          await holdActiveMedia(victim, {
            renderer: true,
            suppressClientCleanupRequests: true,
          });
          const baseline = await heldMedia(victim);
          let revoked;
          if (mode === "leave")
            revoked = await api(victim, `/servers/${f.serverId}/leave`, "POST");
          else if (mode === "logout")
            revoked = await api(victim, "/auth/logout", "POST");
          else
            revoked = await api(
              f.owner,
              `/servers/${f.serverId}/${mode}`,
              "POST",
              { user_id: victim.id },
            );
          check(
            revoked.status === 204 || revoked.status === 200,
            "revocation-request-failed",
            { status: revoked.status },
          );
          // Wait a finite revocation budget; then sample a second interval for continued video.
          const settled = await observe(3_000, () => heldMedia(victim));
          const after = await observe(1_000, () => heldMedia(victim));
          const beforeGateway = await rawSample(victim);
          const message = await api(
            f.owner,
            `/channels/${chat}/messages`,
            "POST",
            {
              content: "E2E post-revocation synthetic event",
              attachment_ids: [],
            },
          );
          check(
            message.status === 201 || message.status === 200,
            "revocation-event-fixture-failed",
            { status: message.status },
          );
          const gateway = await observe(1_000, () => rawSample(victim));
          const rest = await api(victim, `/channels/${chat}/messages`);
          const freshTicket = await api(
            victim,
            `/channels/${voice}/media-ticket`,
            "POST",
          );
          // Evaluate every boundary, even when one is red; do not short circuit away the ticket attack.
          let held;
          try {
            held = await useHeldTicket(victim);
          } catch (error) {
            if (error instanceof NativeInterfaceFailure) throw error;
            held = {
              accepted: error.metrics?.accepted ?? null,
              denied: error.metrics?.denied ?? false,
              failed: true,
            };
          }
          const metrics = {
            controlConfirmed:
              after.frames > settled.frames ||
              gateway.events > beforeGateway.events ||
              held.accepted === true ||
              rest.status === 200 ||
              freshTicket.status === 200,
            gatewayControl,
            revocationStatus: revoked.status,
            baseline,
            settled,
            after,
            beforeGateway,
            gateway,
            restStatus: rest.status,
            freshTicketStatus: freshTicket.status,
            heldTicket: held,
            clientTeardownSuppressed: true,
          };
          check(
            after.frames === settled.frames &&
              after.openSockets === 0 &&
              gateway.events === beforeGateway.events &&
              [401, 403, 404].includes(rest.status) &&
              [401, 403, 404].includes(freshTicket.status) &&
              !held.failed,
            "revoked-client-retained-access",
            metrics,
          );
          return metrics;
        } finally {
          await closeAccessActors([victim], [victim]);
        }
      },
    );
  }
  await h.run(
    "logout-other-independent-session-survives",
    ["03a"],
    async () => {
      const context = await deadlineProbe(
        () => h.browser.newContext(),
        Date.now() + 12_000,
      );
      let page;
      try {
        page = await deadlineProbe(() => context.newPage(), Date.now() + 5_000);
        await page.goto(`${f.base}/login`);
        await page.getByLabel("E-Mail-Adresse").fill(f.owner.email);
        await page
          .getByLabel("Passwort", { exact: true })
          .fill(f.owner.password);
        await page
          .getByRole("button", { name: "Anmelden", exact: true })
          .click();
        await page.waitForURL((u) => !u.pathname.includes("login"));
        const before = await api({ page }, "/auth/session");
        check(
          before.status === 200 && before.body?.user?.id === f.owner.id,
          "fixture-independent-session-not-authenticated",
          { status: before.status },
        );
        const ownerLogout = await api(f.owner, "/auth/logout", "POST");
        check(ownerLogout.status === 200, "owner-logout-request-failed", {
          status: ownerLogout.status,
        });
        const ownerSession = await api(f.owner, "/auth/session");
        check(
          ownerSession.status === 200 && ownerSession.body?.user === null,
          "owner-session-not-anonymous-after-logout",
          { status: ownerSession.status },
        );
        const after = await api({ page }, "/auth/session");
        check(
          after.status === 200 && after.body?.user?.id === f.owner.id,
          "logout-killed-other-session",
          { status: after.status },
        );
        return {
          independentSessionRetained: true,
          ownerLogoutStatus: ownerLogout.status,
          ownerSessionAnonymousStatus: ownerSession.status,
          independentSessionStatus: after.status,
        };
      } finally {
        await closeIndependentSession(page, context);
      }
    },
  );
  await h.run(
    "channel-server-delete-active-sockets",
    ["03a", "03b"],
    async () => {
      const controls = [];
      for (const scope of ["channel", "server"]) {
        // A separate owned fixture prevents server deletion contaminating later cases.
        const owned = await h.fixture();
        const victim = owned.watcher;
        const voiceId = owned.voicePath.split("/").at(-1);
        const chatId = owned.textPath.split("/").at(-1);
        try {
          await click(owned.owner, "Beitreten");
          await click(owned.owner, "Go Live");
          await click(victim, "Zuschauen");
          await progress(victim, {
            relay: h.relay,
            budget: h.relay ? 10_000 : 5_000,
          });
          await maliciousGateway(victim, owned.serverId, chatId);
          if (scope === "server") {
            const before = await rawSample(victim);
            const seed = await api(
              owned.owner,
              `/channels/${chatId}/messages`,
              "POST",
              { content: "E2E delete-server native control" },
            );
            check(seed.status === 201, "fixture-delete-server-control-failed");
            await until(
              () => rawSample(victim),
              (s) => s.events > before.events,
              "fixture-delete-server-event-not-observed",
            );
          }
          await heldTicket(victim, voiceId);
          const rendererControl = await holdActiveMedia(victim, {
            renderer: true,
          });
          const removed =
            scope === "server"
              ? await api(owned.owner, `/servers/${owned.serverId}`, "DELETE")
              : await api(owned.owner, `/channels/${voiceId}`, "DELETE");
          check(removed.status === 204, "fixture-owned-room-delete-failed", {
            status: removed.status,
          });
          if (scope === "server") h.markServerDeleted(owned.serverId);
          const settled = await observe(3_000, () => heldMedia(victim));
          const after = await observe(1_000, () => heldMedia(victim));
          const fresh = await api(
            victim,
            `/channels/${voiceId}/media-ticket`,
            "POST",
          );
          const held = await useHeldTicket(victim);
          check(
            after.frames === settled.frames &&
              after.openSockets === 0 &&
              [403, 404].includes(fresh.status) &&
              !held.accepted,
            "deleted-room-retained-active-media-or-ticket-access",
            {
              controlConfirmed:
                after.frames > settled.frames ||
                fresh.status === 200 ||
                held.accepted,
              scope,
              settled,
              after,
              freshTicketStatus: fresh.status,
              heldTicket: held,
            },
          );
          if (scope === "server") {
            const rest = await api(victim, `/channels/${chatId}/messages`);
            check(
              [403, 404].includes(rest.status),
              "deleted-server-retained-chat-access",
              { controlConfirmed: true, status: rest.status },
            );
          } else {
            // Deleting one voice channel must preserve the remaining text channel.
            check(
              (await api(victim, `/channels/${chatId}/messages`)).status ===
                200,
              "channel-delete-revoked-unrelated-text-channel",
            );
          }
          controls.push({
            scope,
            counterMode: rendererControl.counterMode,
            rendererControl,
            settled,
            after,
            deleteStatus: removed.status,
            activeDecodedControl: true,
            clientTeardownSuppressed: true,
            framesStopped: true,
            mediaSocketClosed: true,
            heldTicketRejected: true,
            freshTicketStatus: fresh.status,
          });
        } finally {
          await closeAccessActors(
            [owned.owner, owned.member, victim],
            [victim],
          );
        }
      }
      return { controls };
    },
  );
  h.setIsolation(null);
}
