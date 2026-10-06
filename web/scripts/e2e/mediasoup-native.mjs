/* global window, document, MessageEvent, URL, process, fetch, AbortSignal */
// Short actual-product checks. No load matrix, long soak or physical devices.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { open } from "node:fs/promises";
import { isAbsolute } from "node:path";
import { setTimeout as pause } from "node:timers/promises";
import { startHarness, check, click, until, observe } from "./harness.mjs";
import { nativeEvaluate } from "./native-evaluate.mjs";
import { progress } from "./media.mjs";
import { holdActiveMedia, heldMedia, releaseHeld } from "./access.mjs";
import { receiveLossProxy } from "./receive-loss-proxy.mjs";

// Only a child created here can be stopped. No external process/PID control.
async function ownMediaRuntime() {
  const binary = process.env.GELABBER_NATIVE_MEDIA_BINARY;
  if (!binary) return null;
  assert.ok(isAbsolute(binary));
  const ready = new URL(`http://${process.env.MEDIA_ADDR}/ready`);
  const redis = new URL(process.env.REDIS_URL);
  assert.equal(ready.hostname, "127.0.0.1");
  assert.equal(redis.hostname, "127.0.0.1");
  assert.equal(
    new URL(`udp://${process.env.MEDIA_ICE_BIND}`).hostname,
    "127.0.0.1",
  );
  const log = await open(process.env.GELABBER_NATIVE_MEDIA_LOG, "wx", 0o600);
  let child;
  const stop = async (signal = "SIGTERM") => {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const current = child;
    current.kill(signal);
    const waitForExit = () =>
      until(
        () =>
          Promise.resolve({
            code: current.exitCode,
            signal: current.signalCode,
          }),
        (s) => s.code !== null || s.signal !== null,
        "owned-media-process-stop-deadline",
        20_000,
      );
    try {
      await waitForExit();
    } catch (error) {
      current.kill("SIGKILL");
      await waitForExit();
      throw error;
    }
    if (signal === "SIGTERM")
      check(current.exitCode === 0, "owned-media-process-shutdown-not-normal", {
        code: current.exitCode,
        signal: current.signalCode,
      });
  };
  const start = async () => {
    child = spawn(binary, [], {
      env: process.env,
      stdio: ["ignore", log.fd, log.fd],
    });
    await until(
      async () => {
        check(
          child.exitCode === null && child.signalCode === null,
          "owned-media-process-exited-before-ready",
        );
        try {
          const response = await fetch(ready, {
            signal: AbortSignal.timeout(1000),
          });
          const body = await response.json();
          return (
            response.ok &&
            body.status === "ready" &&
            body.checks.redis.status === "ok" &&
            body.checks.media.status === "ok"
          );
        } catch {
          return false;
        }
      },
      (value) => value,
      "owned-media-process-readiness-deadline",
      20_000,
    );
  };
  try {
    await start();
  } catch (error) {
    await stop().finally(() => log.close());
    throw error;
  }
  return {
    crashAndRestart: async (afterStop) => {
      await stop("SIGKILL");
      await pause(1000);
      await afterStop();
      await start();
    },
    close: async () => {
      await stop().finally(() => log.close());
    },
  };
}

function installReceiverProbe({ proxyPort, mediaPort }) {
  window.addEventListener("gelabber:media-consumer", ({ detail }) => {
    const state = window.__e2e;
    if (detail.receiver.track !== detail.track)
      throw new Error("NATIVE_CONSUMER_BINDING");
    state.nativeSources ??= [];
    state.nativeSources.push(detail);
  });
  const Socket = window.WebSocket;
  window.WebSocket = class extends Socket {
    constructor(url, protocols) {
      super(url, protocols);
      const media = new URL(url, window.location.href).pathname === "/media/ws";
      const add = this.addEventListener.bind(this);
      this.addEventListener = (type, listener, options) => {
        if (!media || type !== "message" || typeof listener !== "function")
          return add(type, listener, options);
        return add(
          type,
          (event) => {
            const frame = JSON.parse(event.data);
            const state = window.__e2e;
            state.nativeFrames ??= [];
            state.nativeFrames.push({ op: frame.op, k: frame.k ?? null });
            if (state.nativeFrames.length > 32) state.nativeFrames.shift();
            if (
              frame.op === "consumer" &&
              frame.k === "l" &&
              state.holdLiveAnnouncement
            ) {
              state.heldLiveAnnouncements ??= [];
              if (state.heldLiveAnnouncements.length >= 2)
                throw new Error("HELD_LIVE_ANNOUNCEMENT_BOUND_EXCEEDED");
              state.heldLiveAnnouncements.push(() =>
                listener.call(this, event),
              );
              return;
            }
            if (
              proxyPort &&
              frame.op === "result" &&
              Array.isArray(frame.data?.iceCandidates)
            ) {
              for (const candidate of frame.data.iceCandidates) {
                if (
                  candidate.protocol !== "udp" ||
                  candidate.address !== "127.0.0.1" ||
                  candidate.port !== mediaPort
                )
                  throw new Error("OWNED_LOOPBACK_CANDIDATE_REQUIRED");
                candidate.port = proxyPort;
              }
              event = new MessageEvent("message", {
                data: JSON.stringify(frame),
              });
            }
            listener.call(this, event);
          },
          options,
        );
      };
      const send = this.send.bind(this);
      let heldReady = false;
      const afterReady = [];
      this.send = (data) => {
        const state = window.__e2e;
        const frame = JSON.parse(data);
        if (media && frame.op === "produce") {
          state.nativeProducerParameters ??= new Map();
          state.nativeProducerParameters.set(frame.k, frame.rtp);
        }
        const deliver = () => {
          if (media && frame.op === "q") {
            state.layerPreferences ??= [];
            state.layerPreferences.push({ ...frame });
            if (state.layerPreferences.length > 32)
              state.layerPreferences.shift();
          }
          return send(data);
        };
        if (heldReady) {
          if (afterReady.length >= 63)
            throw new Error("HELD_RPC_BOUND_EXCEEDED");
          afterReady.push(deliver);
          return;
        }
        if (media && frame.op === "consumerReady" && state.holdConsumerReady) {
          const source = state.nativeSources?.find(
            (s) =>
              s.consumerId === frame.consumerId &&
              s.generation === frame.generation &&
              s.k === "l",
          );
          if (source) {
            state.heldConsumerReady ??= [];
            if (state.heldConsumerReady.length >= 2)
              throw new Error("HELD_READY_BOUND_EXCEEDED");
            heldReady = true;
            state.heldConsumerReady.push(() => {
              heldReady = false;
              deliver();
              // IDs stay unchanged and retain their original socket order.
              for (const sendNext of afterReady.splice(0)) sendNext();
            });
            return;
          }
        }
        return deliver();
      };
    }
  };
}
async function sourceState(actor, owner) {
  return nativeEvaluate(
    actor,
    async (owner) => {
      const state = window.__e2e;
      const live = [...state.nativeSources]
        .reverse()
        .find(
          (s) =>
            s.owner === owner && s.k === "l" && s.track.readyState === "live",
        );
      const mic = [...state.nativeSources]
        .reverse()
        .find(
          (s) =>
            s.owner === owner && s.k === "a" && s.track.readyState === "live",
        );
      if (
        !live ||
        !mic ||
        live.receiver.track !== live.track ||
        mic.receiver.track !== mic.track
      )
        throw new Error("NATIVE_RECEIVER_MISSING");
      const inbound = async (source, kind) => {
        const ssrcs = new Set(
          source.rtpParameters.encodings.map((e) => e.ssrc),
        );
        const rows = [...(await source.receiver.getStats()).values()].filter(
          (r) =>
            r.type === "inbound-rtp" &&
            (r.kind ?? r.mediaType) === kind &&
            ssrcs.has(r.ssrc),
        );
        if (
          rows.length !== 1 ||
          !Number.isFinite(rows[0].packetsReceived) ||
          !Number.isFinite(rows[0].bytesReceived)
        )
          throw new Error("NATIVE_RTP_COUNTER_UNAVAILABLE");
        return rows[0];
      };
      const video = [...document.querySelectorAll("figure video")].find((v) =>
        v.srcObject?.getVideoTracks().includes(live.track),
      );
      if (!video || video.paused || video.readyState < 2)
        throw new Error("NATIVE_VIDEO_NOT_RENDERING");
      const videoStats = await inbound(live, "video"),
        audioStats = await inbound(mic, "audio");
      const preferred = [...(state.layerPreferences ?? [])]
        .reverse()
        .find(
          (p) =>
            p.consumerId === live.consumerId &&
            p.generation === live.generation,
        );
      const layers = state.layers?.get(live.consumerId);
      const frames = video.getVideoPlaybackQuality?.().totalVideoFrames;
      if (!Number.isFinite(frames) || frames <= 0)
        throw new Error("NATIVE_RENDER_COUNTER_UNAVAILABLE");
      return {
        width: video.videoWidth,
        height: video.videoHeight,
        frames,
        videoPackets: videoStats.packetsReceived,
        lost: videoStats.packetsLost,
        audioPackets: audioStats.packetsReceived,
        audioBytes: audioStats.bytesReceived,
        preferred: preferred
          ? { h: preferred.h, congested: preferred.congested }
          : null,
        spatial:
          layers?.generation === live.generation ? layers.spatialLayer : null,
        temporal:
          layers?.generation === live.generation ? layers.temporalLayer : null,
        payloadTypes: live.rtpParameters.codecs.map(
          (codec) => codec.payloadType,
        ),
        captureCalls: state.micCalls + state.cameraCalls + state.displayCalls,
        connectedPeers: state.peers.filter(
          (p) => p.connectionState === "connected",
        ).length,
        qReplies: state.sockets
          .filter((s) => s.plane === "media")
          .flatMap((s) => s.mediaRpc ?? [])
          .filter((r) => r.method === "q")
          .slice(-3),
      };
    },
    owner,
  );
}
async function publisherState(actor) {
  return nativeEvaluate(actor, async () => {
    const producer = window.__e2e.outgoingSources.get("l");
    if (!producer || producer.sender.track !== producer.track)
      throw new Error("NATIVE_PUBLISHER_BINDING");
    const parameters = producer.sender.getParameters();
    const encodings = parameters.encodings;
    const rids = new Set(encodings.map((e) => e.rid));
    const settings = producer.track.getSettings();
    const report = await producer.pc.getStats();
    const input = window.__e2e.nativeProducerParameters?.get("l");
    return {
      width: settings.width,
      height: settings.height,
      encodings: encodings.map((e) => ({
        scale: e.scaleResolutionDownBy,
        active: e.active,
        maxBitrate: e.maxBitrate ?? null,
      })),
      senderHeaders: parameters.headerExtensions.map((e) => ({
        uri: e.uri,
        id: e.id,
      })),
      producerHeaders:
        input?.headerExtensions?.map((e) => ({ uri: e.uri, id: e.id })) ?? null,
      feedback:
        input?.codecs
          ?.filter((c) => c.mimeType.toLowerCase() === "video/vp8")
          .flatMap((c) => c.rtcpFeedback) ?? null,
      bandwidth: [...report.values()]
        .filter(
          (r) =>
            r.type === "candidate-pair" &&
            r.state === "succeeded" &&
            r.nominated,
        )
        .map((r) => ({
          outgoing: r.availableOutgoingBitrate ?? null,
          rtt: r.currentRoundTripTime ?? null,
        })),
      rows: [...(await producer.sender.getStats()).values()]
        .filter(
          (r) =>
            r.type === "outbound-rtp" &&
            (r.kind ?? r.mediaType) === "video" &&
            rids.has(r.rid),
        )
        .map((r) => ({
          rid: r.rid,
          packets: r.packetsSent,
          frames: r.framesEncoded,
          width: r.frameWidth,
          height: r.frameHeight,
          quality: r.qualityLimitationReason ?? null,
          targetBitrate: r.targetBitrate ?? null,
        })),
    };
  });
}
async function renderSize(actor, height) {
  await nativeEvaluate(
    actor,
    (height) => {
      for (const video of document.querySelectorAll("figure video")) {
        video.style.setProperty("height", `${height}px`, "important");
        video.style.setProperty("width", `${(height * 16) / 9}px`, "important");
        video.style.setProperty("max-height", "none", "important");
        video.style.setProperty("max-width", "none", "important");
      }
    },
    height,
  );
}
const runtime = await ownMediaRuntime();
let h;
try {
  h = await startHarness();
} catch (error) {
  await runtime?.close();
  throw error;
}
const proxies = [];
try {
  let f;
  const setup = await h.setup(
    "isolated-mediasoup-native-fixture",
    [],
    async () => {
      f = await h.fixture();
      const mediaPort = Number(process.env.GELABBER_MEDIASOUP_TEST_UDP_PORT);
      const proxy = await receiveLossProxy({ port: mediaPort });
      proxies.push(proxy);
      await f.member.page.reload();
      await f.watcher.page.reload();
      // Playwright does not promise ordering between separate init scripts.
      // Install this adapter after the common probe, before any media join.
      await f.member.page.evaluate(installReceiverProbe, {
        proxyPort: 0,
        mediaPort,
      });
      await f.watcher.page.evaluate(installReceiverProbe, {
        proxyPort: proxy.port,
        mediaPort,
      });
      await f.owner.page.evaluate(installReceiverProbe, {
        proxyPort: 0,
        mediaPort,
      });
      await click(f.owner, "Beitreten");
      await until(
        () =>
          nativeEvaluate(f.owner, async () => {
            const state = window.__e2e;
            const producer = state.outgoingSources.get("a");
            const ssrcs = new Set(producer?.encodings?.map((e) => e.ssrc));
            const rows = producer
              ? [...(await producer.sender.getStats()).values()].filter(
                  (r) =>
                    r.type === "outbound-rtp" &&
                    (r.kind ?? r.mediaType) === "audio" &&
                    ssrcs.has(r.ssrc),
                )
              : [];
            return {
              present: !!producer,
              bound: producer?.sender.track === producer?.track,
              connection: producer?.pc?.connectionState ?? null,
              packets: rows.length === 1 ? rows[0].packetsSent : null,
              rpc: state.sockets
                .filter((s) => s.plane === "media")
                .flatMap((s) => s.mediaRpc ?? [])
                .slice(-12),
            };
          }),
        (s) =>
          s.present &&
          s.bound &&
          s.connection === "connected" &&
          Number.isFinite(s.packets) &&
          s.packets > 10,
        "native-publisher-mic-fixture-not-established",
      );
      await click(f.owner, "Go Live");
      await click(f.member, "Zuschauen");
      await nativeEvaluate(f.watcher, () => {
        window.__e2e.holdConsumerReady = true;
        window.__e2e.holdLiveAnnouncement = true;
      });
      await click(f.watcher, "Zuschauen");
      await progress(f.member);
      // Backend publication iteration is unordered. Deliver the original Live
      // announcement only after real Mic Ready/RTP, without forging a response.
      await until(
        () =>
          nativeEvaluate(f.watcher, async () => {
            const state = window.__e2e;
            const mic = state.nativeSources?.find((s) => s.k === "a");
            const rows =
              mic && mic.receiver.track === mic.track
                ? [...(await mic.receiver.getStats()).values()].filter(
                    (r) =>
                      r.type === "inbound-rtp" &&
                      (r.kind ?? r.mediaType) === "audio" &&
                      mic.rtpParameters.encodings.some(
                        (e) => e.ssrc === r.ssrc,
                      ),
                  )
                : [];
            return {
              sources: state.nativeSources?.map((s) => s.k) ?? [],
              packets: rows.length === 1 ? rows[0].packetsReceived : null,
              heldAnnouncements: state.heldLiveAnnouncements?.length ?? 0,
              heldReady: state.heldConsumerReady?.length ?? 0,
              peers: state.peers.map((pc) => pc.connectionState),
              frames: state.nativeFrames ?? [],
              rpc: state.sockets
                .filter((s) => s.plane === "media")
                .flatMap((s) => s.mediaRpc ?? [])
                .slice(-12),
            };
          }),
        (value) => Number.isFinite(value.packets) && value.packets > 10,
        "native-mic-not-established-before-held-live-ready",
      );
      await nativeEvaluate(f.watcher, () => {
        const state = window.__e2e;
        state.holdLiveAnnouncement = false;
        for (const deliver of state.heldLiveAnnouncements?.splice(0) ?? [])
          deliver();
      });
      return {
        accounts: 3,
        viewers: 2,
        ownedUdpAdapter: true,
        syntheticCapture: true,
      };
    },
  );
  if (setup.status === "PASS") {
    await h.run("consumer-paused-until-browser-ready", [], async () => {
      const payloadTypes = await until(
        () =>
          nativeEvaluate(f.watcher, () => {
            const state = window.__e2e;
            const live = state.nativeSources?.find((s) => s.k === "l");
            return live &&
              state.heldConsumerReady?.length === 1 &&
              live.receiver.track === live.track &&
              live.track.readyState === "live"
              ? live.rtpParameters.codecs.map((c) => c.payloadType)
              : null;
          }),
        (p) => Array.isArray(p) && p.length > 0,
        "native-consumer-not-installed-before-ready",
      );
      const proxy = proxies[0];
      proxy.setVideoPayloadTypes(payloadTypes);
      const audioPackets = () =>
        nativeEvaluate(f.watcher, async () => {
          const mic = window.__e2e.nativeSources?.find((s) => s.k === "a");
          if (!mic || mic.receiver.track !== mic.track) return 0;
          const ssrcs = new Set(mic.rtpParameters.encodings.map((e) => e.ssrc));
          const rows = [...(await mic.receiver.getStats()).values()].filter(
            (r) =>
              r.type === "inbound-rtp" &&
              (r.kind ?? r.mediaType) === "audio" &&
              ssrcs.has(r.ssrc),
          );
          return rows.length === 1 && Number.isFinite(rows[0].packetsReceived)
            ? rows[0].packetsReceived
            : 0;
        });
      const firstAudio = await until(
        audioPackets,
        (p) => p > 10,
        "native-audio-positive-control-missing",
      );
      const before = proxy.snapshot();
      const after = await observe(1000, () => proxy.snapshot());
      check(
        after.forwarded > before.forwarded && after.video === 0,
        "paused-native-consumer-sent-video-before-ready",
        { before, after },
      );
      check(
        (await audioPackets()) > firstAudio,
        "native-audio-did-not-advance-while-video-paused",
      );
      await progress(f.member);
      await nativeEvaluate(f.watcher, () => {
        const state = window.__e2e;
        state.holdConsumerReady = false;
        for (const deliver of state.heldConsumerReady.splice(0)) deliver();
      });
      await progress(f.watcher);
      check(
        proxy.snapshot().video > 0,
        "resumed-native-consumer-video-missing",
      );
      return {
        installedNativeReceiver: true,
        before,
        after,
        resumed: proxy.snapshot(),
      };
    });
    await h.run("independent-viewer-layers-with-receive-loss", [], async () => {
      await renderSize(f.member, 70);
      await renderSize(f.watcher, 300);
      const small = await until(
        () => sourceState(f.member, f.owner.id),
        (s) => s.width === 160 && s.spatial === 0 && s.preferred?.h <= 90,
        "small-render-layer-not-confirmed",
        15000,
      );
      const full = await until(
        async () => ({
          ...(await sourceState(f.watcher, f.owner.id)),
          publisher: await publisherState(f.owner),
        }),
        (s) =>
          s.width === 640 &&
          s.spatial === 1 &&
          s.preferred?.h > 90 &&
          !s.preferred.congested,
        "full-render-layer-not-confirmed",
        15000,
      );
      check(
        small.captureCalls === 0 &&
          full.captureCalls === 0 &&
          small.connectedPeers === 1 &&
          full.connectedPeers === 1,
        "watcher-captured-or-created-send-peer",
      );
      const proxy = proxies[0];
      proxy.setVideoPayloadTypes(full.payloadTypes);
      proxy.setLoss(true);
      const degraded = await until(
        () => sourceState(f.watcher, f.owner.id),
        (s) =>
          s.preferred?.congested === true &&
          s.spatial === 0 &&
          s.width === 160 &&
          s.frames > full.frames &&
          s.audioPackets > full.audioPackets &&
          proxy.snapshot().dropped > 20,
        "actual-receiver-loss-did-not-select-low-layer",
        20000,
      );
      const unaffected = await sourceState(f.member, f.owner.id);
      const loss = proxy.snapshot();
      check(
        loss.dropped > 20 && loss.foreign === 0,
        "selected-video-loss-not-observed",
        loss,
      );
      check(
        Number.isFinite(degraded.lost) && degraded.lost > full.lost,
        "native-receiver-loss-counter-not-advanced",
      );
      check(
        unaffected.spatial === 0 &&
          unaffected.frames > small.frames &&
          unaffected.audioPackets > small.audioPackets &&
          !unaffected.preferred.congested,
        "other-viewer-interrupted-or-congested",
      );
      proxy.setLoss(false);
      const recovered = await until(
        () => sourceState(f.watcher, f.owner.id),
        (s) =>
          !s.preferred?.congested &&
          s.spatial === 1 &&
          s.width === 640 &&
          s.frames > degraded.frames &&
          s.audioPackets > degraded.audioPackets,
        "receiver-layer-did-not-recover-after-healthy-hysteresis",
        20000,
      );
      await renderSize(f.member, 300);
      const independent = await until(
        () => sourceState(f.member, f.owner.id),
        (s) => s.spatial === 1 && s.width === 640,
        "other-viewer-could-not-independently-change-layer",
        15000,
      );
      return {
        small,
        full,
        degraded,
        unaffected,
        recovered,
        independent,
        loss,
        limitation:
          "owned loopback UDP adapter drops one viewer video RTP; no load or WAN claim",
      };
    });
    if (runtime)
      await h.run(
        "native-media-process-failure-preserves-capture",
        [],
        async () => {
          const before = await nativeEvaluate(f.owner, () => {
            const state = window.__e2e;
            state.savedNativeCapture = state.captures.find(
              (c) =>
                c.kind === "display" &&
                c.track.kind === "video" &&
                c.track.readyState === "live",
            ).track;
            return {
              displayCalls: state.displayCalls,
              micCalls: state.micCalls,
            };
          });
          await runtime.crashAndRestart(async () => {
            await until(
              () =>
                nativeEvaluate(f.watcher, () =>
                  window.__e2e.peers.every(
                    (pc) => pc.connectionState === "closed",
                  ),
                ),
              (value) => value,
              "old-native-transports-not-closed-after-process-failure",
            );
            proxies[0].resetClient();
          });
          await progress(f.member, { budget: 20_000 });
          await progress(f.watcher, { budget: 20_000 });
          const after = await nativeEvaluate(f.owner, () => {
            const state = window.__e2e;
            return {
              displayCalls: state.displayCalls,
              micCalls: state.micCalls,
              captureLive: state.savedNativeCapture.readyState === "live",
              captureStillSent: state.peers.some(
                (pc) =>
                  pc.connectionState === "connected" &&
                  pc
                    .getSenders()
                    .some((s) => s.track === state.savedNativeCapture),
              ),
            };
          });
          check(
            after.displayCalls === before.displayCalls &&
              after.micCalls === before.micCalls &&
              after.captureLive &&
              after.captureStillSent,
            "native-media-failure-recaptured-or-lost-track",
            { before, after },
          );
          return {
            ownedEngineProcessKilled: true,
            restartedReady: true,
            before,
            after,
          };
        },
      );
    await h.run("watch-off-held-receiver-stops-native-media", [], async () => {
      try {
        let baseline;
        try {
          baseline = await holdActiveMedia(f.watcher, { renderer: true });
        } catch (error) {
          // Surface only the helper's fixed guard codes, never Playwright's
          // URLs, user identities, call logs or arbitrary exception text.
          const code = /\b(E2E_HELD_[A-Z_]+)\b/.exec(error?.message ?? "")?.[1];
          if (code) {
            const receivers = await nativeEvaluate(f.watcher, async () => {
              const state = window.__e2e;
              const rows = [];
              for (const pc of state.heldPeers ?? []) {
                for (const receiver of pc.getReceivers()) {
                  if (receiver.track.kind !== "video") continue;
                  const sources = (state.incomingTracks ?? []).filter(
                    (source) =>
                      source.pc === pc &&
                      source.receiver === receiver &&
                      source.track === receiver.track,
                  );
                  const report = [
                    ...(await receiver.getStats()).values(),
                  ].filter(
                    (row) =>
                      row.type === "inbound-rtp" &&
                      (row.kind ?? row.mediaType) === "video",
                  );
                  const parameters = receiver.getParameters();
                  rows.push({
                    decoder: {
                      mids: pc
                        .getTransceivers()
                        .filter(
                          (transceiver) => transceiver.receiver === receiver,
                        )
                        .map((transceiver) => transceiver.mid),
                      encodings:
                        parameters.encodings?.map((encoding) => ({
                          probatorSsrc: encoding.ssrc === 1234,
                          ssrcAvailable: Number.isInteger(encoding.ssrc),
                        })) ?? null,
                      codecs:
                        parameters.codecs?.map((codec) => ({
                          mime: codec.mimeType,
                          payload: codec.payloadType,
                        })) ?? null,
                    },
                    probator: receiver.track.id === "probator",
                    live: receiver.track.readyState === "live",
                    sourceBindings: sources.length,
                    kinds: sources.map((source) => source.sourceKind),
                    nativeRows: report.length,
                    native: report.map((row) => ({
                      probatorSsrc: row.ssrc === 1234,
                      packets: row.packetsReceived ?? null,
                      bytes: row.bytesReceived ?? null,
                      frames: row.framesDecoded ?? null,
                    })),
                  });
                }
              }
              return rows;
            });
            check(false, code, { receivers });
          }
          throw error;
        }
        const control = await observe(1000, () => heldMedia(f.watcher));
        check(
          control.frames > baseline.frames,
          "held-receiver-negative-control-not-advancing",
        );
        const unaffected = await sourceState(f.member, f.owner.id);
        const buttons = f.watcher.page.getByRole("button", {
          name: /Zuschauen beenden|Nicht mehr zuschauen|Zuschauen stoppen/,
        });
        check((await buttons.count()) > 0, "watch-off-ui-action-missing");
        await buttons.first().click();
        const settled = await observe(3000, () => heldMedia(f.watcher));
        const after = await observe(1000, () => heldMedia(f.watcher));
        check(
          after.frames === settled.frames &&
            after.frameCountersAvailable &&
            after.counters.every(
              (c) => c.binding.liveTrack && c.binding.enabledTrack,
            ),
          "watch-off-held-receiver-still-rendering",
          { settled, after },
        );
        const other = await sourceState(f.member, f.owner.id);
        check(
          other.frames > unaffected.frames &&
            other.audioPackets > unaffected.audioPackets,
          "watch-off-interrupted-other-viewer",
        );
        return { baseline, control, settled, after, other };
      } finally {
        await releaseHeld(f.watcher);
      }
    });
  }
} catch (error) {
  await h.setup("mediasoup-native-interrupted", [], async () => {
    throw error;
  });
} finally {
  if (runtime) {
    // Browser-interface failure must never skip cleanup of our own child.
    try {
      await runtime.close();
      h.report.results.push({
        id: "owned-media-runtime-normal-shutdown",
        setup: true,
        status: "PASS",
        metrics: { normalShutdown: true },
      });
    } catch {
      h.report.results.push({
        id: "owned-media-runtime-normal-shutdown",
        setup: true,
        status: "FAIL",
        reason: "owned-media-runtime-shutdown-failed",
      });
      process.exitCode = 1;
    }
  }
  for (const proxy of proxies)
    await proxy.close().catch(() => {
      process.exitCode = 1;
    });
  await h.finish();
}
