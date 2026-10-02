// Two isolated Chromium contexts; capture is synthetic, API/gateway/SFU and RTP
// are real. No Docker, netem, forced TURN, native picker or physical audio claim.
// Run: GELABBER_SOURCE_AUDIO_URL=http://127.0.0.1:5173 node scripts/smoke-source-audio.mjs
/* global process, window, document, navigator, fetch, setInterval, clearInterval, console, URL */
import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { setTimeout as pause } from "node:timers/promises";
import { chromium } from "playwright";

const target = new URL(
  process.env.GELABBER_SOURCE_AUDIO_URL ?? "http://127.0.0.1:5173",
);
assert.ok(["http:", "https:"].includes(target.protocol));
assert.ok(
  !target.username && !target.password && !target.search && !target.hash,
);
assert.ok(
  ["127.0.0.1", "localhost", "[::1]"].includes(target.hostname),
  "Source-audio smoke requires a loopback test stack",
);
const base = target.origin;
const suffix = `${Date.now()}-${process.pid}`;
const password = `Source-${suffix}-password`;
const actors = [];
const report = {
  status: "running",
  capture: "synthetic canvas video and oscillator audio",
  network: "actual API, gateway, SFU and native WebRTC RTP",
  limitations: [
    "No physical system/tab audio capture or native display-picker coverage",
    "No TURN, WAN or audible two-device coverage",
  ],
  checks: [],
  media: [],
  cleanup: {},
};
let stage = "launch";
let server;
let owner;
const browser = await chromium.launch({
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
  ],
});

function instrument() {
  const state = (window.__sourceAudioSmoke = {
    peers: [],
    incoming: new Map(),
    elements: new Set(),
    captures: [],
    microphones: new Set(),
    rtp: new Map(),
    controls: [],
    micCalls: 0,
    noAudioNext: false,
  });
  const Peer = window.RTCPeerConnection;
  window.RTCPeerConnection = class extends Peer {
    constructor(config) {
      super(config);
      state.peers.push(this);
      this.addEventListener("track", (event) => {
        state.incoming.set(event.track, event.streams[0]?.id ?? "");
      });
    }
  };
  const play = window.HTMLMediaElement.prototype.play;
  window.HTMLMediaElement.prototype.play = function (...args) {
    state.elements.add(this);
    return play.apply(this, args);
  };
  const Socket = window.WebSocket;
  window.WebSocket = class extends Socket {
    constructor(url, protocols) {
      super(url, protocols);
      this.isMedia =
        new URL(url, window.location.href).pathname === "/media/ws";
      if (this.isMedia)
        this.addEventListener("message", (event) => {
          const frame = JSON.parse(event.data);
          state.controls.push({
            direction: "in",
            op: frame.op,
            error: frame.e,
            version: frame.v,
          });
        });
    }
    send(data) {
      if (this.isMedia) {
        const frame = JSON.parse(data);
        if (["j", "p", "u", "w"].includes(frame.op))
          state.controls.push({
            direction: "out",
            op: frame.op,
            kind: frame.k,
            on: frame.on,
            track: !!frame.t,
            parent: !!frame.parent,
            claim: !!frame.lc,
            version: frame.v,
          });
      }
      return super.send(data);
    }
  };
  const gum = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
  navigator.mediaDevices.getUserMedia = async (constraints) => {
    const stream = await gum(constraints);
    if (constraints.audio) {
      state.micCalls++;
      stream.getAudioTracks().forEach((track) => state.microphones.add(track));
    }
    return stream;
  };
  navigator.mediaDevices.getDisplayMedia = async (constraints) => {
    const canvas = document.createElement("canvas");
    canvas.width = 640;
    canvas.height = 360;
    const ctx = canvas.getContext("2d");
    let frame = 0;
    const draw = () => {
      ctx.fillStyle = frame++ % 2 ? "#266ef1" : "#cf4f62";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
    };
    draw();
    const timer = setInterval(draw, 80);
    const stream = canvas.captureStream(12);
    const video = stream.getVideoTracks()[0];
    const stopVideo = video.stop.bind(video);
    video.stop = () => {
      clearInterval(timer);
      stopVideo();
    };
    let audio = null;
    if (constraints.audio && !state.noAudioNext) {
      const context = new window.AudioContext();
      const destination = context.createMediaStreamDestination();
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      oscillator.frequency.value = 523.25;
      gain.gain.value = 0.12;
      oscillator.connect(gain).connect(destination);
      oscillator.start();
      await context.resume();
      audio = destination.stream.getAudioTracks()[0];
      const stopAudio = audio.stop.bind(audio);
      audio.stop = () => {
        stopAudio();
        if (context.state !== "closed") {
          oscillator.stop();
          void context.close();
        }
      };
      stream.addTrack(audio);
    }
    state.noAudioNext = false;
    state.captures.push({ stream, video, audio, constraints });
    return stream;
  };
}

async function sample() {
  const state = window.__sourceAudioSmoke;
  const parse = (id) => {
    const match = /^(.*):(sa|la|s|l|a|v)(?:[-:].*)?$/.exec(id ?? "");
    return match ? { publisher: match[1], kind: match[2] } : null;
  };
  const received = [];
  const receivedIdentity = new Map();
  const sent = [];
  const remember = (key, value) => {
    const previous = state.rtp.get(key);
    state.rtp.set(key, {
      ...value,
      packets: Math.max(previous?.packets ?? 0, value.packets),
    });
  };
  for (const [index, pc] of state.peers.entries()) {
    if (pc.connectionState === "closed") continue;
    for (const receiver of pc.getReceivers()) {
      const track = receiver.track;
      const mid = pc
        .getTransceivers()
        .find((item) => item.receiver === receiver)?.mid;
      const section = pc.remoteDescription?.sdp
        .split(/\r?\nm=/)
        .find((item) => item.split(/\r?\n/).includes(`a=mid:${mid}`));
      const msid = section
        ?.split(/\r?\n/)
        .find((line) => line.startsWith("a=msid:"))
        ?.slice(7)
        .split(" ");
      // Native receiver track IDs remain immutable when an SFU sender/MID is
      // reused. The currently negotiated MSID and SSRC bind the actual source.
      const identity =
        parse(msid?.[1]) ??
        parse(msid?.[0]) ??
        parse(state.incoming.get(track)) ??
        parse(track.id);
      const currentSsrcs = new Set(
        section
          ?.split(/\r?\n/)
          .flatMap((line) => /^a=ssrc:(\d+)/.exec(line)?.[1] ?? []) ?? [],
      );
      const stats = await receiver.getStats();
      const inbound = [...stats.values()].filter(
        (stat) =>
          stat.type === "inbound-rtp" && currentSsrcs.has(String(stat.ssrc)),
      );
      const packets = inbound.reduce(
        (sum, stat) => sum + (stat.packetsReceived ?? 0),
        0,
      );
      const row = {
        track: track.id,
        publisher: identity?.publisher ?? null,
        kind: identity?.kind ?? track.kind,
        streamKind: parse(state.incoming.get(track))?.kind ?? null,
        advertisedKind:
          parse(msid?.[1])?.kind ?? parse(msid?.[0])?.kind ?? null,
        packets,
        decodedSamples: inbound.reduce(
          (sum, stat) =>
            sum +
            (stat.totalSamplesReceived ?? stat.jitterBufferEmittedCount ?? 0),
          0,
        ),
        audioEnergy: inbound.reduce(
          (sum, stat) => sum + (stat.totalAudioEnergy ?? 0),
          0,
        ),
        connected: pc.connectionState === "connected",
        live: track.readyState === "live",
      };
      received.push(row);
      receivedIdentity.set(track, row.kind);
      for (const stat of inbound)
        remember(`in:${index}:${track.id}:${stat.id}`, {
          direction: "in",
          publisher: row.publisher,
          kind: row.kind,
          packets: stat.packetsReceived ?? 0,
        });
    }
    for (const sender of pc.getSenders()) {
      const track = sender.track;
      if (!track) continue;
      const capture = state.captures.find(
        (item) => item.video === track || item.audio === track,
      );
      const kind = state.microphones.has(track)
        ? "mic"
        : capture?.audio === track
          ? "source-audio"
          : capture?.video === track
            ? "source-video"
            : track.kind;
      const stats = await sender.getStats();
      const outbound = [...stats.values()].filter(
        (stat) => stat.type === "outbound-rtp",
      );
      sent.push({
        track: track.id,
        kind,
        enabled: track.enabled,
        live: track.readyState === "live",
        packets: outbound.reduce(
          (sum, stat) => sum + (stat.packetsSent ?? 0),
          0,
        ),
      });
    }
  }
  const playback = [...state.elements]
    .filter((el) => el.tagName === "AUDIO" && el.srcObject)
    .map((el) => ({
      source: el.dataset.sourceAudio ?? null,
      publisher: el.dataset.publisher ?? null,
      connection: el.dataset.connection ?? null,
      paused: el.paused,
      muted: el.muted,
      volume: el.volume,
      tracks: el.srcObject.getAudioTracks().map((track) => ({
        id: track.id,
        kind: receivedIdentity.get(track) ?? parse(track.id)?.kind ?? "mic",
        live: track.readyState === "live",
      })),
    }));
  const videos = [...document.querySelectorAll("figure")].map((figure) => {
    const video = figure.querySelector("video");
    return {
      label: figure.querySelector("figcaption")?.textContent ?? "",
      attached: !!video?.srcObject,
      width: video?.videoWidth ?? 0,
      muted: video?.muted ?? true,
    };
  });
  const capture = state.captures.at(-1);
  return {
    received,
    sent,
    playback,
    videos,
    micCalls: state.micCalls,
    captureCount: state.captures.length,
    capture: capture
      ? {
          wantedAudio: !!capture.constraints.audio,
          constraints: capture.constraints.audio,
          videoLive: capture.video.readyState === "live",
          audioLive: capture.audio?.readyState === "live",
          audioTrack: capture.audio?.id ?? null,
          videoTrack: capture.video.id,
        }
      : null,
    counters: [...state.rtp.values()],
    controls: state.controls.slice(-30),
  };
}

const snapshot = (actor) => actor.page.evaluate(sample);
const packets = (sample, kinds, publisher) =>
  sample.counters
    .filter(
      (row) =>
        kinds.includes(row.kind) && (!publisher || row.publisher === publisher),
    )
    .reduce((sum, row) => sum + row.packets, 0);
const sourcePlayback = (sample, kind) =>
  sample.playback.filter((el) => el.source === kind);
const micPlayback = (sample) =>
  sample.playback.filter((el) => !el.source && el.tracks.length > 0);
const dock = (actor) => actor.page.locator(".voice-session-dock");
const tile = (actor, kind) =>
  actor.page.locator("figure").filter({
    has: actor.page.locator("figcaption", {
      hasText: kind === "s" ? "— Bildschirm" : "— Live",
    }),
  });

async function until(probe, accept, message, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    const value = await probe();
    last = value;
    if (accept(value)) return value;
    await pause(Math.min(100, Math.max(0, deadline - Date.now())));
  }
  if (last?.received)
    report.lastObservation = {
      received: last.received.map(
        ({
          kind,
          streamKind,
          advertisedKind,
          packets,
          decodedSamples,
          audioEnergy,
          connected,
          live,
        }) => ({
          kind,
          streamKind,
          advertisedKind,
          packets,
          decodedSamples,
          audioEnergy,
          connected,
          live,
        }),
      ),
      sent: last.sent.map(({ kind, packets, enabled, live }) => ({
        kind,
        packets,
        enabled,
        live,
      })),
      playback: last.playback.map(
        ({ source, paused, muted, volume, tracks }) => ({
          source,
          paused,
          muted,
          volume,
          kinds: tracks.map((track) => track.kind),
        }),
      ),
      controls: last.controls,
    };
  throw new assert.AssertionError({ message });
}

// Negative claims need a bounded observation interval, with native packets
// sampled throughout. This must run while the positive microphone path flows.
async function observe(probe, check, duration = 1_200) {
  const deadline = Date.now() + duration;
  do {
    check(await probe());
    await pause(Math.min(100, Math.max(0, deadline - Date.now())));
  } while (Date.now() < deadline);
}

async function api(actor, path, method = "GET", body) {
  return actor.page.evaluate(
    async ({ path, method, body }) => {
      const session = await fetch("/api/auth/session").then((r) => r.json());
      const response = await fetch(`/api${path}`, {
        method,
        headers: {
          "Content-Type": "application/json",
          "X-CSRF-Token": session.csrf_token,
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      });
      return {
        status: response.status,
        body: response.status === 204 ? null : await response.json(),
      };
    },
    { path, method, body },
  );
}

async function participant(name) {
  const context = await browser.newContext({
    permissions: ["camera", "microphone"],
  });
  await context.addInitScript(instrument);
  const actor = { context, page: await context.newPage(), name };
  actors.push(actor);
  actor.page.setDefaultTimeout(15_000);
  await actor.page.goto(`${base}/register`);
  await actor.page.getByLabel("Name", { exact: true }).fill(name);
  await actor.page
    .getByLabel("E-Mail-Adresse")
    .fill(`source-audio-${suffix}-${actors.length}@example.test`);
  await actor.page.getByLabel("Passwort", { exact: true }).fill(password);
  await actor.page
    .getByRole("button", { name: "Registrieren", exact: true })
    .click();
  await actor.page.waitForURL((url) => !url.pathname.includes("register"));
  actor.id = (await api(actor, "/auth/session")).body?.user?.id;
  assert.ok(actor.id, "Registration must create an authenticated account");
  return actor;
}

async function setVolume(actor, label, percent) {
  const slider = dock(actor).getByRole("slider", { name: label, exact: true });
  await slider.press("Home");
  for (let value = 0; value < percent; value++)
    await slider.press("ArrowRight");
}

async function progress(actor, kinds, publisher) {
  const before = packets(await snapshot(actor), kinds, publisher);
  return until(
    () => snapshot(actor),
    (state) => packets(state, kinds, publisher) > before + 10,
    `${kinds.join("+")} RTP packets must advance`,
  );
}

function noLoopback(state, self) {
  assert.ok(
    state.received.every(
      (row) => row.packets === 0 || (row.publisher && row.publisher !== self),
    ),
    "The SFU must never send a participant its own source or microphone",
  );
  assert.ok(
    state.playback.every((el) => el.publisher !== self),
    "Local capture must never create source-audio playback",
  );
  assert.ok(
    state.videos.every((video) => video.muted),
    "Video elements stay muted",
  );
  const playing = state.playback.flatMap((el) =>
    el.paused
      ? []
      : el.tracks.filter((track) => track.live).map((track) => track.id),
  );
  assert.equal(
    new Set(playing).size,
    playing.length,
    "Each received audio track must have exactly one playback path",
  );
}

async function stopForwarding(actor, kinds, publisher) {
  let previous;
  let stableSince = Date.now();
  const settled = await until(
    () => snapshot(actor),
    (state) => {
      const current = packets(state, kinds, publisher);
      if (current !== previous) stableSince = Date.now();
      previous = current;
      return Date.now() - stableSince >= 800;
    },
    `${kinds.join("+")} forwarding must stop after bounded packet drain`,
  );
  const baseline = packets(settled, kinds, publisher);
  const micBefore = packets(settled, ["a"], publisher);
  await observe(
    () => snapshot(actor),
    (state) => {
      assert.equal(
        packets(state, kinds, publisher),
        baseline,
        "Source packets after stop",
      );
      assert.equal(
        state.playback.filter((el) => el.source).length,
        0,
        "Source playback after stop",
      );
    },
  );
  await until(
    () => snapshot(actor),
    (state) => packets(state, ["a"], publisher) > micBefore + 10,
    "Microphone RTP must remain healthy after source stop",
  );
}

async function watch(receiver, kind) {
  await tile(receiver, kind)
    .getByRole("button", { name: /^(Zuschauen|Nicht mehr zuschauen)$/ })
    .waitFor();
  // An explicit Watch intent survives a publisher stopping/restarting a source.
  if (
    await tile(receiver, kind)
      .getByRole("button", { name: "Nicht mehr zuschauen", exact: true })
      .isVisible()
  )
    return;
  await tile(receiver, kind)
    .getByRole("button", { name: "Zuschauen", exact: true })
    .click();
}

async function playbackControls(receiver, publisher, kind) {
  const audioKind = `${kind}a`;
  await setVolume(receiver, "Wiedergabe-Lautstärke", 25);
  await setVolume(receiver, "Stream-Ton-Lautstärke", 65);
  const expected = (state, muted = false, deafened = false) =>
    sourcePlayback(state, kind).length === 1 &&
    sourcePlayback(state, kind).every(
      (el) =>
        el.volume === (deafened ? 0 : 0.65) && el.muted === (deafened || muted),
    ) &&
    micPlayback(state).length > 0 &&
    micPlayback(state).every(
      (el) => el.volume === (deafened ? 0 : 0.25) && el.muted === deafened,
    );
  await until(
    () => snapshot(receiver),
    expected,
    "Independent voice/source volume",
  );
  await dock(receiver)
    .getByRole("button", { name: "Stream-Ton aus", exact: true })
    .click();
  await until(
    () => snapshot(receiver),
    (state) => expected(state, true),
    "Source mute must preserve voice playback",
  );
  await progress(receiver, [audioKind], publisher.id);
  await dock(receiver)
    .getByRole("button", { name: "Stream-Ton an", exact: true })
    .click();
  await until(() => snapshot(receiver), expected, "Source unmute");
  await dock(publisher)
    .getByRole("button", { name: "Mikrofon aus", exact: true })
    .click();
  await until(
    () => snapshot(publisher),
    (state) =>
      state.sent.some((row) => row.kind === "mic" && !row.enabled) &&
      state.sent.some((row) => row.kind === "source-audio" && row.enabled),
    "Microphone mute must preserve source sender",
  );
  await progress(receiver, [audioKind], publisher.id);
  await dock(publisher)
    .getByRole("button", { name: "Mikrofon an", exact: true })
    .click();
  await dock(receiver)
    .getByRole("button", { name: "Taub stellen", exact: true })
    .click();
  await until(
    () => snapshot(receiver),
    (state) =>
      expected(state, false, true) &&
      state.sent.some((row) => row.kind === "mic" && !row.enabled),
    "Deafen must mute every audio path and microphone",
  );
  await dock(receiver)
    .getByRole("button", { name: "Hören", exact: true })
    .click();
  await until(
    () => snapshot(receiver),
    (state) =>
      expected(state) &&
      state.sent.some((row) => row.kind === "mic" && row.enabled),
    "Undeafen must restore both volumes and microphone",
  );
  report.checks.push(
    `${kind}: independent volume, source/microphone mute, deafen restoration`,
  );
}

async function sourceScenario(receiver, kind) {
  stage = `${kind}: start`;
  const start = kind === "s" ? "Bildschirm teilen" : "Go Live";
  const stop = kind === "s" ? "Teilen beenden" : "Live beenden";
  const audioKind = `${kind}a`;
  const capturesBefore = (await snapshot(owner)).captureCount;
  await dock(owner).getByRole("button", { name: start, exact: true }).click();
  const publisher = await until(
    () => snapshot(owner),
    (state) =>
      state.captureCount === capturesBefore + 1 &&
      state.sent.some(
        (row) => row.kind === "source-audio" && row.packets > 10,
      ) &&
      state.sent.some((row) => row.kind === "source-video" && row.packets > 10),
    "Source video and audio must each send real RTP",
  );
  assert.ok(publisher.capture.wantedAudio, "Capture must request source audio");
  assert.equal(publisher.capture.constraints.echoCancellation, false);
  assert.equal(publisher.capture.constraints.noiseSuppression, false);
  assert.equal(publisher.capture.constraints.autoGainControl, false);
  assert.equal(publisher.capture.constraints.channelCount, 2);
  assert.ok(
    publisher.sent.some(
      (row) => row.kind === "mic" && row.track !== publisher.capture.audioTrack,
    ),
    "Source audio must have a sender distinct from the microphone",
  );
  noLoopback(publisher, owner.id);
  stage = `${kind}: no forwarding before Watch`;
  await tile(receiver, kind)
    .getByRole("button", { name: "Zuschauen", exact: true })
    .waitFor();
  await observe(
    () => snapshot(receiver),
    (state) => {
      assert.equal(
        packets(state, [kind, audioKind], owner.id),
        0,
        "No source RTP before Watch",
      );
      assert.equal(
        sourcePlayback(state, kind).length,
        0,
        "No source playback before Watch",
      );
    },
  );
  await progress(receiver, ["a"], owner.id);
  stage = `${kind}: Watch`;
  await watch(receiver, kind);
  await progress(receiver, [kind], owner.id);
  const watched = await progress(receiver, [audioKind], owner.id);
  assert.ok(
    watched.received.some(
      (row) => row.kind === audioKind && row.connected && row.packets > 0,
    ),
    "Source receiver must carry actual RTP",
  );
  const decoded = await until(
    () => snapshot(receiver),
    (state) =>
      state.received.some(
        (row) =>
          row.kind === audioKind &&
          row.decodedSamples > 0 &&
          row.audioEnergy > 0,
      ),
    "Source audio must decode actual non-silent oscillator samples",
  );
  report.media.push({
    source: kind,
    videoPackets: packets(decoded, [kind], owner.id),
    sourceAudioPackets: packets(decoded, [audioKind], owner.id),
    microphonePackets: packets(decoded, ["a"], owner.id),
    sourceDecodedSamples: decoded.received
      .filter((row) => row.kind === audioKind)
      .reduce((sum, row) => sum + row.decodedSamples, 0),
    sourceAudioEnergy: decoded.received
      .filter((row) => row.kind === audioKind)
      .reduce((sum, row) => sum + row.audioEnergy, 0),
  });
  assert.ok(
    watched.received.some((row) => row.kind === "a" && row.packets > 0),
    "Microphone receiver remains distinct",
  );
  await until(
    () => snapshot(receiver),
    (state) =>
      sourcePlayback(state, kind).length === 1 &&
      sourcePlayback(state, kind).every(
        (el) =>
          !el.paused &&
          el.tracks.length === 1 &&
          el.tracks[0].kind === audioKind &&
          el.publisher === owner.id &&
          el.connection === "voice",
      ) &&
      state.videos.some(
        (video) =>
          video.label.includes(kind === "s" ? "— Bildschirm" : "— Live") &&
          video.attached &&
          video.width > 0,
      ),
    "Watched source must render video and one matching audio track",
  );
  noLoopback(await snapshot(receiver), receiver.id);
  stage = `${kind}: playback controls`;
  await playbackControls(receiver, owner, kind);
  stage = `${kind}: Stop Watch`;
  await tile(receiver, kind)
    .getByRole("button", { name: "Nicht mehr zuschauen", exact: true })
    .click();
  await stopForwarding(receiver, [kind, audioKind], owner.id);
  stage = `${kind}: re-Watch and parent stop`;
  await watch(receiver, kind);
  await progress(receiver, [audioKind], owner.id);
  await dock(owner).getByRole("button", { name: stop, exact: true }).click();
  await until(
    () => snapshot(owner),
    (state) =>
      !state.capture.videoLive &&
      !state.capture.audioLive &&
      state.sent.every(
        (row) => row.kind !== "source-audio" && row.kind !== "source-video",
      ),
    "Parent stop must end source audio and video senders",
  );
  await stopForwarding(receiver, [kind, audioKind], owner.id);
  report.checks.push(
    `${kind}: actual A/V RTP, Watch/Stop Watch gating, no loopback, parent cleanup`,
  );
}

try {
  stage = "fixtures";
  owner = await participant("Source Publisher");
  const receiver = await participant("Source Receiver");
  const created = await api(owner, "/servers", "POST", {
    name: `Source Audio ${suffix}`,
  });
  assert.equal(created.status, 201, "Create isolated community");
  server = created.body.id;
  assert.ok(server, "Create community must return its identifier");
  const channel = await api(owner, `/servers/${server}/channels`, "POST", {
    name: "Source Audio",
    kind: "voice",
  });
  assert.equal(channel.status, 201, "Create isolated voice channel");
  const invite = await api(owner, `/servers/${server}/invites`, "POST", {
    max_uses: 1,
    expires_in_hours: 1,
  });
  assert.equal(invite.status, 201, "Create fixture invite");
  const joined = await api(
    receiver,
    `/invites/${invite.body.code}/join`,
    "POST",
  );
  assert.equal(joined.status, 200, "Join fixture community");
  const voiceUrl = `${base}/s/${server}/c/${channel.body.id}`;
  for (const actor of [owner, receiver]) {
    await actor.page.goto(voiceUrl);
    await actor.page
      .getByRole("button", { name: "Beitreten", exact: true })
      .click();
    await until(
      () => snapshot(actor),
      (state) =>
        state.sent.some(
          (row) =>
            row.kind === "mic" && row.live && row.enabled && row.packets > 10,
        ),
      "Real microphone sender must be established",
    );
  }
  await progress(owner, ["a"], receiver.id);
  await progress(receiver, ["a"], owner.id);
  await dock(owner).getByLabel("Ton teilen", { exact: true }).check();
  await sourceScenario(receiver, "s");
  await sourceScenario(receiver, "l");

  stage = "no-audio fallback";
  const sourceBefore = packets(await snapshot(receiver), ["sa"], owner.id);
  await owner.page.evaluate(() => {
    window.__sourceAudioSmoke.noAudioNext = true;
  });
  await dock(owner)
    .getByRole("button", { name: "Bildschirm teilen", exact: true })
    .click();
  await owner.page
    .getByRole("status")
    .filter({
      hasText:
        "Der Browser hat keinen Stream-Ton freigegeben. Das Video läuft weiter.",
    })
    .waitFor();
  const silentCapture = (await snapshot(owner)).capture;
  assert.ok(
    silentCapture.wantedAudio &&
      silentCapture.videoLive &&
      !silentCapture.audioLive,
    "Requested audio may be unavailable without losing video",
  );
  stage = "no-audio fallback: Watch";
  await watch(receiver, "s");
  stage = "no-audio fallback: video and audio absence";
  await progress(receiver, ["s"], owner.id);
  await until(
    () => snapshot(receiver),
    (state) =>
      state.videos.some(
        (video) =>
          video.label.includes("— Bildschirm") &&
          video.attached &&
          video.width > 0,
      ),
    "No-audio fallback must still render video",
  );
  await observe(
    () => snapshot(receiver),
    (state) => {
      assert.equal(
        packets(state, ["sa"], owner.id),
        sourceBefore,
        "Fallback must not fabricate source audio",
      );
      assert.equal(
        sourcePlayback(state, "s").length,
        0,
        "Fallback has no source audio playback",
      );
    },
  );
  await dock(owner)
    .getByRole("button", { name: "Teilen beenden", exact: true })
    .click();
  await stopForwarding(receiver, ["s", "sa"], owner.id);
  report.checks.push(
    "no-audio fallback: video, visible notice, no fabricated source audio",
  );

  stage = "listen-only Live Watch";
  await dock(receiver)
    .getByRole("button", { name: "Verlassen", exact: true })
    .click();
  await until(
    () => snapshot(receiver),
    (state) => state.sent.length === 0 && state.playback.length === 0,
    "Leaving voice must clear microphone and playback",
  );
  const micCalls = (await snapshot(receiver)).micCalls;
  await dock(owner)
    .getByRole("button", { name: "Go Live", exact: true })
    .click();
  await receiver.page
    .locator(".voice-room-watch-actions")
    .getByRole("button", { name: "Zuschauen", exact: true })
    .click();
  await progress(receiver, ["la"], owner.id);
  const listenOnly = await until(
    () => snapshot(receiver),
    (state) =>
      sourcePlayback(state, "l").some(
        (el) => el.connection === "watch" && !el.paused,
      ) &&
      state.videos.some(
        (video) =>
          video.label.includes("— Live") && video.attached && video.width > 0,
      ),
    "Listen-only Watch must play selected Live video/source audio",
  );
  assert.equal(
    listenOnly.micCalls,
    micCalls,
    "Listen-only Watch must not acquire a microphone",
  );
  assert.equal(
    listenOnly.sent.length,
    0,
    "Listen-only Watch must not send capture",
  );
  assert.ok(
    listenOnly.received.every(
      (row) => ["l", "la", "a"].includes(row.kind) || row.packets === 0,
    ),
    "Watch source media must belong to the selected Live; room microphones remain available",
  );
  assert.ok(
    sourcePlayback(listenOnly, "l").every(
      (el) => el.tracks.length === 1 && el.tracks[0].kind === "la",
    ),
    "Watch source audio must remain separate from room microphones",
  );
  noLoopback(listenOnly, receiver.id);
  await dock(receiver)
    .getByRole("button", { name: "Nicht mehr zuschauen", exact: true })
    .click();
  await until(
    () => snapshot(receiver),
    (state) => state.received.length === 0 && state.playback.length === 0,
    "Stop Watch must close listen-only peer and audio",
  );
  report.checks.push(
    "listen-only Live Watch: selected A/V, no microphone acquisition, complete stop",
  );
  await dock(owner)
    .getByRole("button", { name: "Live beenden", exact: true })
    .click();
  report.status = "passed";
} catch (error) {
  report.status = "failed";
  // Playwright errors may include form values/call logs; retain only a safe code.
  report.failure = {
    stage,
    reason: error instanceof assert.AssertionError ? error.message : error.name,
  };
  process.exitCode = 1;
} finally {
  if (server && owner) {
    try {
      const deleted = await api(owner, `/servers/${server}`, "DELETE");
      report.cleanup.community = deleted.status;
      assert.equal(deleted.status, 204, "Delete owned fixture community");
    } catch {
      report.cleanup.community = "failed";
      process.exitCode = 1;
    }
  }
  // There is no account-deletion endpoint. Remove owned content and sessions;
  // isolated example.test accounts remain, matching the existing E2E harness.
  report.cleanup.accounts =
    "example.test fixtures retained; no account-deletion API";
  report.cleanup.logout = [];
  for (const actor of actors) {
    try {
      const loggedOut = await api(actor, "/auth/logout", "POST");
      report.cleanup.logout.push(loggedOut.status);
      assert.equal(loggedOut.status, 200, "Logout fixture session");
    } catch {
      report.cleanup.logout.push("failed");
      process.exitCode = 1;
    }
    await actor.context.close();
  }
  await browser.close();
  if (process.exitCode) report.status = "failed";
  if (process.env.GELABBER_SOURCE_AUDIO_REPORT)
    await writeFile(
      process.env.GELABBER_SOURCE_AUDIO_REPORT,
      `${JSON.stringify(report, null, 2)}\n`,
    );
  console.log(JSON.stringify(report, null, 2));
}
