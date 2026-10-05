/* global process, console, fetch, performance, URL, AbortSignal */
import assert from "node:assert/strict";
import { readFile, writeFile, readlink } from "node:fs/promises";
import { setTimeout as pause } from "node:timers/promises";
import { startHarness, click, snapshot, until, check } from "./harness.mjs";
import { activePeers, progress } from "./media.mjs";

const duration = Number(process.env.GELABBER_SOAK_SECONDS ?? 3600);
assert.ok(duration >= 30 && duration <= 7200);
const checkpoint =
  process.env.GELABBER_SOAK_CHECKPOINT ??
  "/tmp/gelabber-call-soak-checkpoint.json";
const mediaOrigin = new URL(
  process.env.GELABBER_SOAK_MEDIA_URL ?? "http://127.0.0.1:8081",
);
assert.ok(["http:", "https:"].includes(mediaOrigin.protocol));
assert.ok(["127.0.0.1", "localhost", "[::1]"].includes(mediaOrigin.hostname));
assert.ok(
  !mediaOrigin.username &&
    !mediaOrigin.password &&
    !mediaOrigin.search &&
    !mediaOrigin.hash &&
    mediaOrigin.pathname === "/",
);
const processIds = [
  ["api", process.env.GELABBER_SOAK_API_PID],
  ["media", process.env.GELABBER_SOAK_MEDIA_PID],
].filter(([, pid]) => pid);
for (const [, pid] of processIds) assert.match(pid, /^[1-9][0-9]*$/);
const identities = new Map();
const h = await startHarness();
const resources = async () => {
  const metrics = await fetch(new URL("/metrics", mediaOrigin), {
    signal: AbortSignal.timeout(5000),
  }).then((r) => {
    assert.equal(r.status, 200);
    return r.text();
  });
  const gauge = (name) =>
    Number(new RegExp(`^${name} ([0-9.]+)$`, "m").exec(metrics)?.[1]);
  const processes = {};
  for (const [role, pid] of processIds) {
    const exe = await readlink(`/proc/${pid}/exe`);
    assert.ok(
      exe.endsWith(`/gelabber-${role}`),
      "PID must identify the supplied Gelabber process",
    );
    const stat = await readFile(`/proc/${pid}/stat`, "utf8");
    const startTime = stat.slice(stat.lastIndexOf(")") + 2).split(" ")[19];
    const identity = `${exe}:${startTime}`;
    assert.ok(
      !identities.has(role) || identities.get(role) === identity,
      "PID identity changed during observation",
    );
    identities.set(role, identity);
    const status = await readFile(`/proc/${pid}/status`, "utf8");
    const field = (name) =>
      Number(new RegExp(`^${name}:\\s+(\\d+)`, "m").exec(status)?.[1]);
    processes[role] = { rssKiB: field("VmRSS"), threads: field("Threads") };
  }
  return {
    rooms: gauge("gelabber_media_rooms"),
    peers: gauge("gelabber_media_peers"),
    iceFailures: gauge("gelabber_media_ice_fails_total"),
    processes,
  };
};
const packets = (s) =>
  activePeers(s)
    .flatMap((p) => p.inbound)
    .filter((r) => r.kind === "audio")
    .reduce((n, r) => n + r.packets, 0);
const frames = (s) =>
  activePeers(s)
    .flatMap((p) => p.inbound)
    .filter((r) => r.kind === "video")
    .reduce((n, r) => n + (r.frames ?? 0), 0);
const scalar = (s) => ({
  connected: activePeers(s).filter((p) => p.connection === "connected").length,
  peers: activePeers(s).length,
  audioPackets: packets(s),
  videoFrames: frames(s),
  senders: activePeers(s).reduce((n, p) => n + p.senders, 0),
  transceivers: activePeers(s).reduce((n, p) => n + p.transceivers, 0),
  captures: s.captures.filter((c) => c.state === "live").length,
  playbackTracks: s.playback.reduce(
    (n, p) => n + p.audioTracks + p.videoTracks,
    0,
  ),
  duplicateAudioTracks: s.duplicateAudioPlaybackTracks,
  mediaSockets: s.sockets.filter((s) => s.plane === "media" && s.ready === 1)
    .length,
});
let f;
try {
  await h.setup("isolated-soak-fixture", [], async () => {
    f = await h.fixture();
    h.report.scope = {
      continuousSecondsRequested: duration,
      fakeCapture: true,
      physicalAudio: false,
      network: h.relay ? "native TURN" : "local native ICE",
      serverTelemetry: processIds.length
        ? "operator-supplied PID identity checked across the run"
        : "SFU room/peer/ICE metrics only",
      frozenBuiltWeb: "operator must supply a static candidate without HMR",
    };
    return { actors: 3 };
  });
  await h.run("continuous-call-and-resource-return", [], async () => {
    const baseline = await resources();
    check(
      baseline.rooms === 0 && baseline.peers === 0,
      "fixture-soak-requires-exclusive-media-process",
    );
    await click(f.owner, "Beitreten");
    await click(f.member, "Beitreten");
    await until(
      () => Promise.all([snapshot(f.owner), snapshot(f.member)]),
      (ss) => ss.every((s) => packets(s) > 5),
      "fixture-duplex-not-ready",
      15000,
    );
    await click(f.owner, "Go Live");
    await until(
      () =>
        f.watcher.page
          .getByRole("button", { name: "Zuschauen", exact: true })
          .count(),
      (n) => n > 0,
      "fixture-watch-not-ready",
      10000,
    );
    await click(f.watcher, "Zuschauen");
    await progress(f.watcher, { budget: 15000 });
    let previous = await Promise.all([
      snapshot(f.owner),
      snapshot(f.member),
      snapshot(f.watcher),
    ]).then((ss) => ss.map(scalar));
    const first = previous,
      samples = [];
    const beforeLoop = await resources();
    check(
      beforeLoop.rooms === baseline.rooms + 1 && beforeLoop.peers === 3,
      "fixture-soak-graph-not-exclusive",
      { beforeLoop },
    );
    const start = performance.now();
    while (performance.now() - start < duration * 1000) {
      await pause(
        Math.min(30000, duration * 1000 - (performance.now() - start)),
      );
      const current = await Promise.all([
        snapshot(f.owner),
        snapshot(f.member),
        snapshot(f.watcher),
      ]).then((ss) => ss.map(scalar));
      const resource = await resources();
      for (let i = 0; i < current.length; i++) {
        check(
          current[i].connected > 0 && current[i].peers === first[i].peers,
          "soak-peer-failed-or-grew",
          { sample: current },
        );
        check(
          current[i].transceivers === first[i].transceivers &&
            current[i].senders === first[i].senders &&
            current[i].captures === first[i].captures &&
            current[i].playbackTracks <= first[i].playbackTracks,
          "soak-client-resources-grew",
          { sample: current },
        );
        check(current[i].duplicateAudioTracks === 0, "soak-duplicate-audio", {
          sample: current,
        });
      }
      check(
        current[0].audioPackets > previous[0].audioPackets &&
          current[1].audioPackets > previous[1].audioPackets,
        "soak-duplex-audio-stopped",
        { previous, current },
      );
      check(
        current[2].videoFrames > previous[2].videoFrames,
        "soak-live-video-stopped",
        { previous, current },
      );
      check(resource.iceFailures === baseline.iceFailures, "soak-ice-failed", {
        resource,
      });
      samples.push({
        elapsedSeconds: (performance.now() - start) / 1000,
        clients: current,
        resource,
      });
      previous = current;
      await writeFile(
        checkpoint,
        JSON.stringify({ baseline, first, samples, complete: false }, null, 2),
        { mode: 0o600 },
      );
      console.log(
        `Soak ${Math.floor((performance.now() - start) / 1000)}/${duration}s: duplex audio and live video advancing`,
      );
    }
    await click(f.owner, "Live beenden");
    for (const actor of [f.owner, f.member]) await click(actor, "Verlassen");
    const end = await until(
      async () => ({
        clients: await Promise.all([
          snapshot(f.owner),
          snapshot(f.member),
          snapshot(f.watcher),
        ]).then((ss) => ss.map(scalar)),
        resource: await resources(),
      }),
      (s) =>
        s.resource.rooms === 0 &&
        s.resource.peers === 0 &&
        s.clients.every(
          (c) =>
            c.peers === 0 &&
            c.senders === 0 &&
            c.captures === 0 &&
            c.playbackTracks === 0,
        ),
      "soak-resources-did-not-return",
      15000,
    );
    const metrics = {
      actualContinuousSeconds: samples.at(-1).elapsedSeconds,
      baseline,
      first,
      samples,
      end,
      oneHour: duration >= 3600,
      limitations: [
        "No physical device or audible speech quality",
        "RSS and thread counts are retained as measured; allocator return is not inferred from closed tracks",
        "Other release gates remain separate",
      ],
    };
    await writeFile(
      checkpoint,
      JSON.stringify({ ...metrics, complete: true }, null, 2),
      { mode: 0o600 },
    );
    return metrics;
  });
} finally {
  await h.finish();
}
