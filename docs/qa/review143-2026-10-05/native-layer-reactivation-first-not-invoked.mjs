/* global window */
import { nativeEvaluate } from "./native-evaluate.mjs";

// External diagnostic only. Keep the original failed gate. Mutate only the
// existing native full encoding after a measured no-change window confirms
// live canvas/q progression and no full frames. Never add a track/transceiver,
// create a capture, change caps/codecs or touch application/server state.
export async function diagnoseFullReactivation(actor) {
  return nativeEvaluate(actor, async () => {
    const state = window.__e2e;
    const capture = state.captures.findLast(item => item.track.readyState === "live");
    const pc = state.peers.find(peer => peer.connectionState === "connected" &&
      peer.getSenders().some(sender => sender.track === capture?.track));
    const sender = pc?.getSenders().find(value => value.track === capture?.track);
    const result = { qualified: false, mutation: false, mode: "existing-full-active-pulse",
      samples: [], error: null };
    if (!capture || !pc || !sender) return { ...result, reason: "no-current-owned-source" };
    const track = capture.track, transceivers = pc.getTransceivers().length;
    const displayCalls = state.displayCalls, micCalls = state.micCalls;
    const owned = () => pc.connectionState === "connected" && pc.signalingState === "stable" &&
      sender.track === track && track.readyState === "live" &&
      pc.getSenders().includes(sender) && pc.getTransceivers().length === transceivers;
    const finite = value => Number.isFinite(value) ? value : null;
    const wait = ms => new Promise(resolve => window.setTimeout(resolve, ms));
    const bounded = async promise => {
      let timer;
      try {
        return await Promise.race([promise, new Promise((_, reject) => {
          timer = window.setTimeout(() => reject(new Error("native-control-promise-timeout")), 1500);
        })]);
      } finally { window.clearTimeout(timer); }
    };
    const contract = () => sender.getParameters().encodings.map(encoding => ({
      rid: encoding.rid ?? null, active: encoding.active ?? true,
      maxBitrate: encoding.maxBitrate ?? null, maxFramerate: encoding.maxFramerate ?? null,
      scale: encoding.scaleResolutionDownBy ?? null, priority: encoding.priority ?? null,
    }));
    const read = async phase => {
      if (!owned()) throw new Error("native-control-source-ownership-changed");
      const report = await bounded(sender.getStats());
      if (!owned()) throw new Error("native-control-source-ownership-changed");
      const sample = { at: Date.now(), phase, draws: capture.drawCount?.() ?? null,
        contract: contract(), rtp: [] };
      for (const entry of report.values()) {
        if (entry.type !== "outbound-rtp" || (entry.kind ?? entry.mediaType) !== "video") continue;
        const codec = report.get(entry.codecId);
        sample.rtp.push({ rid: entry.rid ?? null, ssrc: entry.ssrc,
          codec: codec?.mimeType ?? null, pt: codec?.payloadType ?? null,
          frames: finite(entry.framesEncoded), keys: finite(entry.keyFramesEncoded),
          bytes: finite(entry.bytesSent), packets: finite(entry.packetsSent),
          width: finite(entry.frameWidth), height: finite(entry.frameHeight),
          pli: finite(entry.pliCount), fir: finite(entry.firCount), nack: finite(entry.nackCount),
          targetBitrate: finite(entry.targetBitrate), limitation: entry.qualityLimitationReason ?? null });
      }
      result.samples.push(sample); return sample;
    };
    const row = (sample, rid) => sample.rtp.find(entry => entry.rid === rid);
    const progress = (a, b, rid) => {
      const first = row(a, rid), last = row(b, rid);
      return first && last && first.ssrc === last.ssrc &&
        typeof first.frames === "number" && typeof last.frames === "number" && last.frames > first.frames;
    };
    let disabled = false;
    try {
      const before = await read("no-change-before");
      const initial = contract(); result.originalContract = initial;
      if (initial.length !== 2 || !initial.some(e => e.rid === "q" && e.active) ||
        !initial.some(e => e.rid === "f" && e.active)) {
        result.reason = "not-active-qf-contract"; return result;
      }
      await wait(700);
      const noChange = await read("no-change-after");
      const firstFull = row(before, "f"), lastFull = row(noChange, "f");
      const stalledFull = firstFull && lastFull && firstFull.ssrc === lastFull.ssrc &&
        typeof firstFull.frames === "number" && typeof firstFull.bytes === "number" &&
        lastFull.frames === firstFull.frames && lastFull.bytes === firstFull.bytes;
      if (!progress(before, noChange, "q") || !stalledFull || !(noChange.draws > before.draws)) {
        result.reason = "no-stalled-full-positive-q-control"; return result;
      }
      result.qualified = true;
      if (!owned()) throw new Error("native-control-source-ownership-changed");
      if (JSON.stringify(contract()) !== JSON.stringify(initial))
        throw new Error("native-control-encoding-envelope-changed");
      const off = sender.getParameters();
      const full = off.encodings.find(encoding => encoding.rid === "f");
      if (!full) throw new Error("native-control-encoding-envelope-changed");
      full.active = false;
      // Mark before awaiting so finally attempts restoration after a timeout.
      disabled = true; result.mutation = true;
      await bounded(sender.setParameters(off));
      await wait(100);
      if (!owned()) throw new Error("native-control-source-ownership-changed");
      const on = sender.getParameters();
      const restored = on.encodings.find(encoding => encoding.rid === "f");
      if (!restored) throw new Error("native-control-encoding-envelope-changed");
      restored.active = true;
      await bounded(sender.setParameters(on)); disabled = false;
      const start = await read("reactivated-before");
      const deadline = Date.now() + 3500;
      do {
        await wait(100);
        const after = await read("reactivated-after");
        if (progress(start, after, "f")) {
          result.fullProgress = true; break;
        }
      } while (Date.now() < deadline);
      result.fullProgress ??= false;
    } catch (error) { result.error = error.message; }
    finally {
      if (disabled && owned()) {
        try {
          const parameters = sender.getParameters();
          const full = parameters.encodings.find(encoding => encoding.rid === "f");
          if (full) { full.active = true; await bounded(sender.setParameters(parameters)); }
        } catch { result.restoreFailed = true; }
      }
      result.sameCaptureAndSender = owned();
      result.sameCaptureCalls = state.displayCalls === displayCalls && state.micCalls === micCalls;
      result.transceiversBefore = transceivers; result.transceiversAfter = pc.getTransceivers().length;
      result.finalContract = contract();
      result.contractPreserved = JSON.stringify(result.finalContract) === JSON.stringify(result.originalContract);
    }
    return result;
  });
}
