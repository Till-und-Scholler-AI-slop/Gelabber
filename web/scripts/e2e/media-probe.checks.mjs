/* global URL, setTimeout, clearTimeout */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createContext, runInContext } from "node:vm";
import {
  instrument,
  sample,
  sampleVideoConsumers,
  samplePublicationFlow,
} from "./probe.mjs";
import { armPlaybackRetry, watchSource } from "./media.mjs";
import { CheckFailure } from "./harness.mjs";

function watchControlFixture(appears, nativeConsumer = false) {
  let reads = 0,
    clicks = 0;
  const start = {
    count: async () => (++reads >= appears ? 1 : 0),
    first() {
      return this;
    },
    click: async ({ timeout }) => {
      assert.ok(timeout > 0);
      clicks++;
    },
  };
  const tiles = {
    filter() {
      return this;
    },
    getByRole: (_role, { name }) =>
      name === "Zuschauen" ? start : { count: async () => 0 },
  };
  return {
    actor: {
      page: { locator: () => tiles, evaluate: async () => nativeConsumer },
    },
    clicks: () => clicks,
    reads: () => reads,
  };
}

test("Watch action waits for the real source control instead of skipping an asynchronous publication", async () => {
  const f = watchControlFixture(2);
  await watchSource(f.actor, "live", { deadlineEpochMs: Date.now() + 1_000 });
  assert.equal(f.clicks(), 1);
  assert.equal(f.reads(), 2);
});

test("Watch action rejects a missing source within its existing deadline and never clicks a foreign fallback", async () => {
  const f = watchControlFixture(Infinity);
  await assert.rejects(
    watchSource(f.actor, "live", { deadlineEpochMs: Date.now() + 20 }),
    (error) =>
      error instanceof CheckFailure &&
      error.message === "source-watch-control-not-ready",
  );
  assert.equal(f.clicks(), 0);
});

test("autoplay rejection remains armed until a trusted retry button click", () => {
  let listener;
  const state = { rejectPlayback: true };
  const window = {
    __e2e: state,
    addEventListener: (_event, fn, capture) => {
      assert.equal(capture, true);
      listener = fn;
    },
    removeEventListener: (_event, fn, capture) => {
      assert.equal(fn, listener);
      assert.equal(capture, true);
      listener = null;
    },
  };
  runInContext(`(${armPlaybackRetry.toString()})()`, createContext({ window }));
  assert.equal(state.rejectPlayback, true);
  const event = (isTrusted, textContent) => ({
    isTrusted,
    target: { closest: () => ({ textContent }) },
  });
  listener(event(false, "Wiedergabe starten"));
  listener(event(true, "Zuschauen"));
  assert.equal(state.rejectPlayback, true);
  listener(event(true, "Wiedergabe starten"));
  assert.equal(state.rejectPlayback, false);
  assert.equal(listener, null);
});

test("concurrent claim fault holds only native Gateway publish, while SDP rejection still delegates to the browser", async () => {
  const sent = [],
    answers = [];
  const nativeError = Object.assign(new Error("PRIVATE native SDP body"), {
    name: "RTCError",
    errorDetail: "sdp-syntax-error",
  });
  class Peer {
    getSenders() {
      return [{ track: { kind: "video", readyState: "live" } }];
    }
    setRemoteDescription(description) {
      answers.push(description);
      return description.sdp === "valid"
        ? Promise.resolve()
        : Promise.reject(nativeError);
    }
  }
  class Socket {
    constructor(url) {
      this.url = String(url);
    }
    addEventListener() {}
    send(data) {
      sent.push(JSON.parse(data));
    }
  }
  const window = {
    RTCPeerConnection: Peer,
    WebSocket: Socket,
    location: { href: "http://127.0.0.1/" },
    HTMLMediaElement: class {
      play() {
        return Promise.resolve();
      }
    },
  };
  const context = createContext({
    window,
    URL,
    navigator: { mediaDevices: { getUserMedia: async () => {} } },
  });
  runInContext(`(${instrument.toString()})({relay:false})`, context);
  window.__e2e.holdLiveClaims = true;
  const gateway = new window.WebSocket("ws://127.0.0.1/ws");
  const media = new window.WebSocket("ws://127.0.0.1/media/ws");
  const publish = { op: "sig", t: "p", k: "l" };
  gateway.send(JSON.stringify(publish));
  media.send(JSON.stringify(publish));
  gateway.send(JSON.stringify({ ...publish, t: "u" }));
  assert.equal(window.__e2e.heldLiveClaims.length, 1);
  assert.equal(sent.length, 2);
  window.__e2e.holdLiveClaims = false;
  window.__e2e.heldLiveClaims.splice(0)[0]();
  assert.deepEqual(sent.at(-1), publish);
  const peer = new window.RTCPeerConnection();
  window.__e2e.rejectNextVideoAnswer = true;
  await assert.rejects(
    peer.setRemoteDescription({ type: "answer", sdp: "valid" }),
    (error) => error === nativeError,
  );
  await peer.setRemoteDescription({ type: "answer", sdp: "valid" });
  assert.equal(window.__e2e.rejectedSdp, 1);
  assert.equal(answers.length, 2);
  assert.notEqual(answers[0].sdp, "valid");
  assert.equal(answers[1].sdp, "valid");
  assert.deepEqual(JSON.parse(JSON.stringify(window.__e2e.rejectedSdpErrors)), [
    {
      name: "RTCError",
      errorDetail: "sdp-syntax-error",
      isError: false,
      isDOMException: false,
    },
  ]);
  for (let attempt = 0; attempt < 5; attempt++) {
    window.__e2e.rejectNextVideoAnswer = true;
    await assert.rejects(
      peer.setRemoteDescription({ type: "answer", sdp: "valid" }),
      (error) => error === nativeError,
    );
  }
  assert.equal(window.__e2e.rejectedSdpErrors.length, 4);
  assert.ok(
    !JSON.stringify(window.__e2e.rejectedSdpErrors).includes("PRIVATE"),
  );
});

test("room source observations require positive native RTP, deduplicate callbacks, and retain foreign-source failures", async () => {
  const tracks = ["a", "b", "foreign", "placeholder", "video"].map((id) => ({
    id,
    kind: id === "video" ? "video" : "audio",
    readyState: "live",
  }));
  const report = new Map(
    tracks.map((track, i) => [
      String(i),
      {
        type: "inbound-rtp",
        kind: track.kind,
        trackIdentifier: track.id,
        packetsReceived: track.id === "placeholder" ? 0 : 7,
      },
    ]),
  );
  const peer = {
    connectionState: "connected",
    iceConnectionState: "connected",
    getStats: async () => report,
    getTransceivers: () => [],
    getSenders: () => [],
    getReceivers: () => [],
    localDescription: null,
  };
  const incoming = tracks.map((track) => ({
    pc: peer,
    track,
    publisher: track.id === "video" ? "a" : track.id,
    sourceKind: track.kind === "video" ? "l" : "a",
  }));
  const state = {
    peers: [peer],
    incomingTracks: [...incoming, incoming[0]],
    expectedAudioPeer: peer,
    expectedAudioPublishers: ["a", "b"],
    expectedLivePublisher: "a",
    mediaElements: new Set(),
    heldTracks: [],
    heldLiveClaims: [],
    voiceRoster: [],
    sockets: [],
    captures: [],
    renderedVideos: new WeakMap(),
  };
  const context = createContext({
    window: { __e2e: state },
    document: { querySelectorAll: () => [] },
  });
  const read = () => runInContext(`(${sample.toString()})()`, context);
  const first = await read();
  assert.deepEqual([...first.roomAudio.perSource], [1, 1]);
  assert.equal(first.roomAudio.foreign, 1);
  assert.equal(first.watchVideoSources.selectedLive, 1);
  assert.equal(first.watchVideoSources.foreign, 0);
  incoming.at(-1).publisher = "foreign-video";
  const last = await read();
  assert.equal(last.watchVideoSources.selectedLive, 0);
  assert.equal(last.watchVideoSources.foreign, 1);
});

function consumerProbeFixture() {
  const hooks = new Map();
  class Peer {
    receivers = [];
    getReceivers() {
      return this.receivers;
    }
  }
  class Socket {
    listeners = new Map();
    constructor() {}
    addEventListener(type, listener) {
      const listeners = this.listeners.get(type) ?? [];
      listeners.push(listener);
      this.listeners.set(type, listeners);
    }
    send() {}
    message(frame) {
      for (const listener of this.listeners.get("message") ?? [])
        listener({ data: JSON.stringify(frame) });
    }
  }
  const window = {
    RTCPeerConnection: Peer,
    WebSocket: Socket,
    location: { href: "http://127.0.0.1/" },
    addEventListener: (type, listener) => hooks.set(type, listener),
    HTMLMediaElement: class {
      play() {
        return Promise.resolve();
      }
    },
  };
  runInContext(
    `(${instrument.toString()})({relay:false})`,
    createContext({
      window,
      URL,
      navigator: { mediaDevices: { getUserMedia: async () => {} } },
    }),
  );
  return { window, hooks };
}

test("consumer identity binds the exact public SDK receiver and rejects a substituted track or peer", () => {
  const { window, hooks } = consumerProbeFixture();
  const peer = new window.RTCPeerConnection();
  const track = { kind: "audio", readyState: "live" };
  const receiver = { track };
  peer.receivers.push(receiver);
  const detail = {
    track,
    receiver,
    owner: "publisher",
    k: "sa",
    consumerId: "consumer",
    generation: "subscription",
    producerId: "producer",
    epoch: "source-epoch",
    rtpParameters: { encodings: [{ ssrc: 555 }] },
  };
  hooks.get("gelabber:media-consumer")({ detail });
  const recorded = window.__e2e.incomingTracks[0];
  assert.equal(recorded.pc, peer);
  assert.equal(recorded.receiver, receiver);
  assert.equal(recorded.track, track);
  assert.equal(recorded.sourceKind, "sa");
  assert.equal(recorded.rtpParameters, detail.rtpParameters);
  assert.throws(
    () =>
      hooks.get("gelabber:media-consumer")({
        detail: { ...detail, track: { ...track } },
      }),
    /BINDING_UNAVAILABLE/,
  );
  assert.throws(
    () =>
      hooks.get("gelabber:media-consumer")({
        detail: { ...detail, receiver: { track } },
      }),
    /BINDING_UNAVAILABLE/,
  );
});

function nativeVideoInventoryFixture() {
  const video = (ssrc) => ({
    type: "inbound-rtp",
    kind: "video",
    ssrc,
    packetsReceived: 17,
    framesDecoded: ssrc === 1234 ? 0 : 12,
  });
  const track = {
    id: "private-live-track",
    kind: "video",
    readyState: "live",
  };
  const receiver = {
    track,
    getStats: async () => new Map([["live", video(9001)]]),
  };
  const probator = {
    track: { id: "probator", kind: "video", readyState: "live" },
    getStats: async () => new Map([["probator", video(1234)]]),
  };
  const receivers = [receiver, probator];
  const rows = [video(9001), video(1234)];
  const peer = {
    connectionState: "connected",
    getReceivers: () => receivers,
    getStats: async () => new Map(rows.map((row, index) => [index, row])),
  };
  const source = {
    pc: peer,
    track,
    receiver,
    publisher: "private-publisher",
    sourceKind: "l",
    consumerId: "private-consumer",
    producerId: "private-producer",
    epoch: "private-epoch",
    generation: "private-generation",
    rtpParameters: { encodings: [{ ssrc: 9001 }] },
  };
  const state = { peers: [peer], incomingTracks: [source] };
  const context = createContext({
    window: { __e2e: state },
    setTimeout,
    clearTimeout,
  });
  return {
    state,
    peer,
    rows,
    receivers,
    source,
    receiver,
    probator,
    video,
    read: () =>
      runInContext(
        `(${sampleVideoConsumers.toString()})({publisher:"private-publisher",deadlineEpochMs:Date.now()+1000})`,
        context,
      ),
  };
}

test("late Watch inventory binds the actual Consumer SSRC and excludes only the native SDK probation receiver", async () => {
  const f = nativeVideoInventoryFixture();
  const inventory = await f.read();
  assert.deepEqual(JSON.parse(JSON.stringify(inventory)), {
    sources: 1,
    selectedLiveSources: 1,
    foreignSources: 0,
    sourceRtpRows: 1,
    probatorRtpRows: 1,
    unexpectedRtpRows: 0,
    invalidBindings: 0,
    identitiesBound: 1,
  });
  assert.ok(!JSON.stringify(inventory).includes("9001"));
  assert.ok(!JSON.stringify(inventory).includes("private"));
});

test("late Watch rejects an extra stale source instead of allowing two generic video streams", async () => {
  const f = nativeVideoInventoryFixture();
  const track = {
    id: "private-stale-track",
    kind: "video",
    readyState: "live",
  };
  const receiver = {
    track,
    getStats: async () => new Map([["stale", f.video(9002)]]),
  };
  f.receivers.push(receiver);
  f.rows.push(f.video(9002));
  f.state.incomingTracks.push({
    ...f.source,
    track,
    receiver,
    rtpParameters: { encodings: [{ ssrc: 9002 }] },
  });
  const inventory = await f.read();
  assert.equal(inventory.sources, 2);
  assert.equal(inventory.selectedLiveSources, 2);
  assert.equal(inventory.sourceRtpRows, 2);
});

for (const malformed of [
  "unknown",
  "unbound-probator",
  "wrong-probator-ssrc",
]) {
  test(`late Watch cannot exempt an unproved extra RTP stream: ${malformed}`, async () => {
    const f = nativeVideoInventoryFixture();
    if (malformed === "unknown") f.rows.push(f.video(9002));
    else if (malformed === "unbound-probator") f.receivers.pop();
    else {
      f.rows[1] = f.video(1235);
      f.probator.getStats = async () => new Map([["probator", f.video(1235)]]);
    }
    const inventory = await f.read();
    assert.equal(inventory.unexpectedRtpRows, 1);
    assert.equal(inventory.probatorRtpRows, malformed === "unknown" ? 1 : 0);
  });
}

for (const malformed of [
  "missing-parameters",
  "missing-source-identity",
  "receiver-substitution",
  "other-ssrc-only",
  "probator-alias",
  "missing-frames",
]) {
  test(`late Watch actual Consumer binding fails closed: ${malformed}`, async () => {
    const f = nativeVideoInventoryFixture();
    if (malformed === "missing-parameters") delete f.source.rtpParameters;
    else if (malformed === "missing-source-identity") delete f.source.epoch;
    else if (malformed === "receiver-substitution")
      f.source.receiver = { ...f.receiver };
    else if (malformed === "probator-alias")
      f.source.rtpParameters.encodings[0].ssrc = 1234;
    else if (malformed === "missing-frames")
      f.receiver.getStats = async () =>
        new Map([["live", { ...f.video(9001), framesDecoded: undefined }]]);
    else f.receiver.getStats = async () => new Map([["other", f.video(9002)]]);
    const inventory = await f.read();
    assert.equal(inventory.invalidBindings, 1);
    assert.equal(inventory.sourceRtpRows, 0);
  });
}

test("delayed Consumer announcements preserve exact frames and allow reversed delivery without holding gateway or cleanup", () => {
  const { window } = consumerProbeFixture();
  const media = new window.WebSocket("ws://127.0.0.1/media/ws");
  const gateway = new window.WebSocket("ws://127.0.0.1/ws");
  const received = [],
    other = [];
  media.addEventListener("message", (event) =>
    received.push(JSON.parse(event.data)),
  );
  gateway.addEventListener("message", (event) =>
    other.push(JSON.parse(event.data)),
  );
  const audio = {
    op: "consumer",
    consumerId: "audio",
    generation: "a",
    kind: "audio",
  };
  const video = {
    op: "consumer",
    consumerId: "video",
    generation: "v",
    kind: "video",
  };
  window.__e2e.holdTracks = true;
  media.message(audio);
  media.message(video);
  media.message({ op: "result", id: 7, data: {} });
  gateway.message(audio);
  assert.equal(received.length, 1);
  assert.equal(other.length, 1);
  assert.deepEqual(
    [...window.__e2e.heldTracks].map((item) => item.kind),
    ["audio", "video"],
  );
  const held = window.__e2e.heldTracks.splice(0).reverse();
  held.forEach((item) => item.deliver());
  assert.deepEqual(received.slice(1), [video, audio]);
  window.__e2e.holdMediaCleanup = true;
  media.message({ op: "consumerClosed", consumerId: "video", generation: "v" });
  media.message({ op: "err", e: "unauthorized" });
  gateway.message({ op: "err", e: "unauthorized" });
  assert.equal(received.length, 3);
  assert.equal(other.length, 2);
  window.__e2e.holdMediaCleanup = false;
  media.message({ op: "consumerClosed", consumerId: "video", generation: "v" });
  assert.equal(received.length, 4);
});

test("publication diagnostics correlate only actual RPC outcomes and retain no protocol bodies or native IDs", async () => {
  const { window } = consumerProbeFixture();
  const media = new window.WebSocket("ws://127.0.0.1/media/ws");
  media.send(
    JSON.stringify({
      op: "produce",
      id: 1,
      k: "l",
      rtp: "PRIVATE",
      lc: "PRIVATE",
    }),
  );
  media.message({ op: "err", id: 1, e: "forbidden" });
  media.send(
    JSON.stringify({ op: "w", id: 2, k: "l", on: true, u: "PRIVATE" }),
  );
  media.message({ op: "result", id: 2, data: { producerId: "PRIVATE" } });
  const flow = await runInContext(
    `(${samplePublicationFlow.toString()})({kind:"l"})`,
    createContext({ window }),
  );
  assert.equal(flow.producer.present, false);
  assert.deepEqual(JSON.parse(JSON.stringify(flow.rpc)), [
    {
      method: "produce",
      kind: "l",
      on: null,
      status: "FAIL",
      error: "forbidden",
    },
    { method: "w", kind: "l", on: true, status: "PASS", error: null },
  ]);
  assert.ok(!JSON.stringify(flow).includes("PRIVATE"));
  for (let id = 3; id < 200; id++)
    media.send(JSON.stringify({ op: "produce", id, k: "l", rtp: "PRIVATE" }));
  assert.equal(window.__e2e.sockets[0].mediaRpc.length, 128);
  assert.equal(window.__e2e.sockets[0].pendingMedia.size, 64);
});
