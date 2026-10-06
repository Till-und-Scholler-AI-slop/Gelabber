import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { setTimeout, clearTimeout } from "node:timers";
import { URL } from "node:url";
import { readFile } from "node:fs/promises";
import { heldMedia, holdActiveMedia, releaseHeld } from "./access.mjs";
import { deadlineProbe, NativeInterfaceFailure } from "./native-evaluate.mjs";
import { check, CheckFailure } from "./harness.mjs";

function consumerSource(pc, receiver, track) {
  return {
    pc,
    receiver,
    track,
    publisher: "00000000-0000-4000-8000-000000000001",
    sourceKind: "l",
    consumerId: "native-consumer",
    producerId: "native-producer",
    epoch: "00000000-0000-4000-8000-000000000002",
    generation: "00000000-0000-4000-8000-000000000003",
    rtpParameters: { encodings: [{ ssrc: 987654321 }] },
  };
}
function consumerIdentity(source) {
  const { publisher, sourceKind, consumerId, producerId, epoch, generation } =
    source;
  return { publisher, sourceKind, consumerId, producerId, epoch, generation };
}

function fixture({ source = "native-video-frame-callback", frames = 20 } = {}) {
  const track = {
    kind: "video",
    readyState: "live",
    enabled: true,
    stop() {
      this.readyState = "ended";
    },
  };
  const receiver = { track };
  const transceiver = {
    receiver,
    mid: "consumer-0",
    currentDirection: "recvonly",
  };
  const pc = {
    getReceivers: () => [receiver],
    getTransceivers: () => [transceiver],
  };
  const publication = consumerSource(pc, receiver, track);
  const stream = { getVideoTracks: () => [track] };
  const cleanup = [];
  const item = {
    source,
    receiver,
    track,
    pc,
    consumerSource: publication,
    frames,
    lastObservedFrames: 0,
    disposed: false,
    callback: 7,
    callbackFunction: () => {},
    video: {
      srcObject: stream,
      getVideoPlaybackQuality: () => ({ totalVideoFrames: item.frames }),
      cancelVideoFrameCallback: (id) => cleanup.push(`cancel:${id}`),
      pause: () => cleanup.push("pause"),
      remove: () => cleanup.push("remove"),
    },
  };
  let statsCalls = 0;
  pc.getStats = async () => {
    statsCalls++;
    return new Map();
  };
  const state = {
    incomingTracks: [publication],
    heldCounterMode: "receiver-renderer",
    heldRenderers: [item],
    heldExpectedRenderers: [
      {
        renderer: item,
        source,
        video: item.video,
        receiver,
        track,
        pc,
        consumerSource: publication,
        consumerIdentity: consumerIdentity(publication),
        consumerSsrc: publication.rtpParameters.encodings[0].ssrc,
        transceiver,
        transceiverMid: transceiver.mid,
        currentDirection: transceiver.currentDirection,
        callbackFunction: item.callbackFunction,
      },
    ],
    heldPeers: [pc],
    heldSockets: [{ ws: { readyState: 3 } }],
  };
  const context = vm.createContext({
    window: { __e2e: state },
    setTimeout,
    clearTimeout,
  });
  const actor = {
    page: {
      evaluate: (fn, arg) =>
        vm.runInContext(`(${fn.toString()})`, context)(arg),
      isClosed: () => false,
    },
  };
  return {
    actor,
    state,
    item,
    cleanup,
    transceiver,
    statsCalls: () => statsCalls,
  };
}

const access = await readFile(new URL("./access.mjs", import.meta.url), "utf8");
const stopCheck = access
  .slice(access.indexOf("channel-server-delete-active-sockets"))
  .match(/check\(\s*after\.frames === settled\.frames[\s\S]*?\n\s*\);/)[0];
function deletionStop(settled, after) {
  vm.runInNewContext(stopCheck, {
    check,
    settled,
    after,
    fresh: { status: 404 },
    held: { accepted: false },
    scope: "channel",
  });
}
const revocationStopCheck = access
  .slice(
    access.indexOf('for (const mode of ["leave", "kick", "ban", "logout"])'),
  )
  .match(/check\(\s*after\.frames === settled\.frames[\s\S]*?\n\s*\);/)[0];
function revocationStop(settled, after) {
  vm.runInNewContext(revocationStopCheck, {
    check,
    settled,
    after,
    gateway: { events: 1 },
    beforeGateway: { events: 1 },
    rest: { status: 404 },
    freshTicket: { status: 404 },
    held: { accepted: false, denied: true, failed: false },
    metrics: {},
  });
}

for (const source of ["native-video-frame-callback", "native-playback-quality"])
  test(`fixed ${source} preserves actual source and detects continuing frames when RTP reports disappear`, async () => {
    const r = fixture({ source });
    const before = await heldMedia(r.actor);
    assert.equal(before.frames, 20);
    r.item.frames = 30;
    const advanced = await heldMedia(r.actor);
    assert.throws(() => deletionStop(before, advanced), CheckFailure);
    const stopped = await heldMedia(r.actor);
    deletionStop(advanced, stopped);
    assert.equal(r.statsCalls(), 0);
    assert.equal(stopped.counters[0].source, source);
  });

for (const mutate of [
  (r) => {
    r.item.frames = null;
  },
  (r) => {
    r.item.frames = NaN;
  },
  (r) => {
    r.item.frames = 19;
  },
  (r) => {
    r.item.receiver.track = { ...r.item.track };
  },
  (r) => {
    r.item.video.srcObject = { getVideoTracks: () => [] };
  },
  (r) => {
    r.state.heldRenderers = [];
  },
  (r) => {
    r.state.heldRenderers = [{ ...r.item }];
  },
  (r) => {
    r.item.source = "native-playback-quality";
  },
  (r) => {
    r.item.disposed = true;
  },
  (r) => {
    r.item.callbackFunction = () => {};
  },
  (r) => {
    r.item.counterFailed = true;
  },
  (r) => {
    r.item.video.paused = true;
  },
  (r) => {
    r.item.video.error = {};
  },
  (r) => {
    r.item.track.readyState = "ended";
  },
  (r) => {
    r.item.track.enabled = false;
  },
  (r) => {
    r.state.incomingTracks = [];
  },
  (r) => {
    r.item.consumerSource = { ...r.item.consumerSource };
  },
  ...[
    "publisher",
    "sourceKind",
    "consumerId",
    "producerId",
    "epoch",
    "generation",
  ].map((key) => (r) => {
    r.item.consumerSource[key] = "replaced-publication";
  }),
  (r) => {
    r.item.consumerSource.rtpParameters.encodings[0].ssrc++;
  },
  (r) => {
    r.item.pc.getReceivers = () => [];
  },
  (r) => {
    r.transceiver.currentDirection = "inactive";
  },
  (r) => {
    r.transceiver.mid = "replacement-mid";
  },
  (r) => {
    r.item.pc.getTransceivers = () => [{ ...r.transceiver }];
  },
  (r) => {
    r.item.pc.getTransceivers = () => [r.transceiver, { ...r.transceiver }];
  },
])
  test(`missing/replaced/reset counter cannot become a stopped-media pass: ${mutate}`, async () => {
    const r = fixture();
    await heldMedia(r.actor);
    mutate(r);
    await assert.rejects(
      heldMedia(r.actor),
      (e) =>
        e instanceof CheckFailure &&
        e.metrics.frames === null &&
        e.metrics.frameCountersAvailable === false,
    );
  });

test("renderer cleanup attempts all resources before a failing peer restore", async () => {
  const r = fixture();
  r.item.video.pause = () => {
    r.cleanup.push("pause");
    throw new Error("PRIVATE-pause");
  };
  r.state.restoreClose = [
    () => {
      r.cleanup.push("restore");
      throw new Error("PRIVATE-restore");
    },
  ];
  await assert.rejects(releaseHeld(r.actor));
  assert.equal(r.item.disposed, true);
  assert.equal(r.item.video.srcObject, null);
  assert.deepEqual(r.cleanup, ["cancel:7", "pause", "remove", "restore"]);
});

function setupFixture({
  playError = false,
  stalled = false,
  appendError = false,
  rescheduleError = false,
  extraReceiver = null,
  sockets = [],
} = {}) {
  const callbacks = new Map(),
    timers = new Map();
  let sequence = 0,
    presented = 0,
    removes = 0,
    plays = 0,
    extraPlays = 0;
  const track = {
    kind: "video",
    readyState: "live",
    enabled: true,
    stop() {
      this.readyState = "ended";
    },
  };
  const receiver = { track };
  const video = {
    style: {},
    requestVideoFrameCallback: (fn) => {
      const id = ++sequence;
      if (rescheduleError && id > 1)
        throw new Error("PRIVATE-callback-scheduling");
      callbacks.set(id, fn);
      if (!stalled)
        timers.set(
          id,
          setTimeout(() => fn(0, { presentedFrames: ++presented }), 5),
        );
      return id;
    },
    cancelVideoFrameCallback: (id) => {
      clearTimeout(timers.get(id));
      timers.delete(id);
    },
    play: async () => {
      plays++;
      if (playError) throw new Error("PRIVATE-native-play");
    },
    pause: () => {},
    remove: () => {
      removes++;
    },
  };
  const peer = {
    connectionState: "connected",
    close() {},
    setRemoteDescription: async () => {},
    setLocalDescription: async () => {},
    getReceivers: () => [receiver, ...(extraReceiver ? [extraReceiver] : [])],
    getTransceivers: () => transceivers,
  };
  const transceivers = [
    { receiver, mid: "consumer-0", currentDirection: "recvonly" },
    ...(extraReceiver ? [{ receiver: extraReceiver, mid: "probator" }] : []),
  ];
  const publication = consumerSource(peer, receiver, track);
  const state = {
    peers: [peer],
    sockets,
    NativeSocket: {
      prototype: {
        close() {
          this.readyState = 3;
        },
      },
    },
    incomingTracks: [publication],
  };
  const context = vm.createContext({
    window: {
      __e2e: state,
      RTCPeerConnection: {
        prototype: {
          close() {
            this.connectionState = "closed";
          },
        },
      },
    },
    document: {
      createElement: () => {
        if (state.heldRenderers.length === 0) return video;
        return {
          style: {},
          play: () => {
            extraPlays++;
            return new Promise(() => {});
          },
          pause() {},
          remove() {},
        };
      },
      body: {
        append() {
          if (appendError) throw new Error("PRIVATE-append");
        },
      },
    },
    MediaStream: class {
      constructor(tracks) {
        this.tracks = tracks;
      }
      getVideoTracks() {
        return this.tracks;
      }
    },
    setTimeout,
    clearTimeout,
  });
  const actor = {
    page: {
      evaluate: (fn, arg) =>
        vm.runInContext(`(${fn.toString()})`, context)(arg),
      isClosed: () => false,
      close: async () => {
        for (const timer of timers.values()) clearTimeout(timer);
      },
    },
  };
  return {
    actor,
    state,
    callbacks,
    removes: () => removes,
    plays: () => plays,
    extraPlays: () => extraPlays,
    peer,
    publication,
    receiver,
    track,
    transceivers,
  };
}

function unmappedReceiver({ id = "probator", rows } = {}) {
  const track = {
    kind: "video",
    id,
    readyState: "live",
    enabled: true,
    stop() {},
  };
  return {
    track,
    getParameters: () => ({
      codecs: [{ mimeType: "video/VP8", payloadType: 127, clockRate: 90_000 }],
    }),
    getStats: async () =>
      new Map(
        (
          rows ?? [
            {
              type: "inbound-rtp",
              kind: "video",
              ssrc: 1234,
              packetsReceived: 7,
              bytesReceived: 999,
              framesDecoded: 0,
            },
          ]
        ).map((row, i) => [`native-report-${i}`, row]),
      ),
  };
}

test("external revocation holds only actual held-media cleanup requests without fabricating replies", async () => {
  const delivered = [],
    unrelatedDelivered = [];
  const send = function (data) {
    delivered.push({ receiver: this, data });
  };
  const ws = { readyState: 1, send, close() {} };
  const gateway = {
    readyState: 1,
    send: (data) => unrelatedDelivered.push(data),
    close() {},
  };
  const r = setupFixture({
    sockets: [
      { plane: "media", ws },
      { plane: "gateway", ws: gateway },
    ],
  });
  try {
    const before = await holdActiveMedia(r.actor, {
      renderer: true,
      suppressClientCleanupRequests: true,
    });
    const cleanupFrames = [
      { op: "l", id: 20 },
      { op: "closeTransport", id: 21, transportId: "owned-transport" },
      {
        op: "consumerFailed",
        id: 22,
        consumerId: r.publication.consumerId,
        generation: r.publication.generation,
      },
    ];
    for (const frame of cleanupFrames) ws.send(JSON.stringify(frame));
    assert.equal(delivered.length, 0);
    assert.equal(ws.readyState, 1);
    const retainedFrames = [
      { op: "consumerReady", id: 23 },
      { op: "q", id: 24, h: 300, congested: false },
      { op: "produce", id: 25 },
      { op: "connect", id: 26 },
      { op: "closeProducer", id: 27 },
      { op: "l", id: 0 },
    ];
    for (const frame of retainedFrames) ws.send(JSON.stringify(frame));
    const binary = new Uint8Array([3]);
    ws.send(binary);
    ws.send("non-JSON native data");
    assert.equal(delivered.length, retainedFrames.length + 2);
    assert.ok(delivered.every((item) => item.receiver === ws));
    assert.equal(delivered.at(-2).data, binary);
    gateway.send(JSON.stringify(cleanupFrames[0]));
    assert.equal(unrelatedDelivered.length, 1);
    await new Promise((resolve) => setTimeout(resolve, 30));
    const after = await heldMedia(r.actor);
    assert.ok(after.frames > before.frames);
    assert.throws(() => revocationStop(before, after), CheckFailure);
    assert.deepEqual(
      JSON.parse(JSON.stringify(after.suppressedClientCleanupRequests)),
      { l: 1, closeTransport: 1, consumerFailed: 1 },
    );
    assert.equal(after.openSockets, 1);
    assert.equal(r.receiver.track, r.track);
    assert.equal(r.track.readyState, "live");
    assert.equal(r.track.enabled, true);
    await releaseHeld(r.actor);
    assert.equal(ws.send, send);
    ws.send(JSON.stringify({ op: "l", id: 28 }));
    assert.equal(delivered.length, retainedFrames.length + 3);
  } finally {
    await releaseHeld(r.actor);
  }
});

test("default held renderer delivers genuine Watch-Off Leave; only explicit external-revoke setup suppresses it", async () => {
  const delivered = [],
    send = (data) => delivered.push(JSON.parse(data));
  const ws = { readyState: 1, send, close() {} };
  const r = setupFixture({ sockets: [{ plane: "media", ws }] });
  try {
    await holdActiveMedia(r.actor, { renderer: true });
    assert.equal(ws.send, send);
    ws.send(JSON.stringify({ op: "l", id: 17 }));
    assert.deepEqual(delivered, [{ op: "l", id: 17 }]);
    const sample = await heldMedia(r.actor);
    assert.deepEqual(
      JSON.parse(JSON.stringify(sample.suppressedClientCleanupRequests)),
      { l: 0, closeTransport: 0, consumerFailed: 0 },
    );
  } finally {
    await releaseHeld(r.actor);
  }
});

test("cleanup diagnostic counts are bounded and original send is restored even after another owned cleanup throws", async () => {
  const send = () => {};
  const ws = { readyState: 1, send, close() {} };
  const r = setupFixture({ sockets: [{ plane: "media", ws }] });
  try {
    await holdActiveMedia(r.actor, {
      renderer: true,
      suppressClientCleanupRequests: true,
    });
    for (let id = 1; id <= 64; id++) ws.send(JSON.stringify({ op: "l", id }));
    assert.throws(
      () => ws.send(JSON.stringify({ op: "l", id: 65 })),
      /E2E_HELD_CLIENT_CLEANUP_OVERFLOW/,
    );
    assert.equal(r.state.heldSuppressedClientCleanupRequests.l, 64);
    r.state.restoreClose.unshift(() => {
      throw new Error("PRIVATE earlier native cleanup failed");
    });
    await assert.rejects(releaseHeld(r.actor), /held-renderer-cleanup-failed/);
    assert.equal(ws.send, send);
    assert.equal(ws.readyState, 3);
    r.state.restoreClose.shift();
  } finally {
    await releaseHeld(r.actor);
  }
});

test("external revocation keeps genuine source-bound renderer progress as its oracle when RTP reports disappear", async () => {
  const r = fixture();
  const before = await heldMedia(r.actor);
  r.item.frames += 4;
  const continuing = await heldMedia(r.actor);
  assert.throws(() => revocationStop(before, continuing), CheckFailure);
  const stopped = await heldMedia(r.actor);
  revocationStop(continuing, stopped);
  assert.equal(r.statsCalls(), 0);
  r.state.heldSockets[0].ws.readyState = 1;
  assert.throws(
    () => revocationStop(stopped, { ...stopped, openSockets: 1 }),
    CheckFailure,
  );
  r.item.frames = null;
  await assert.rejects(heldMedia(r.actor), CheckFailure);
});

test("local SDK receive teardown remains pending without native success and cannot turn ongoing server frames into a privacy pass", async () => {
  const r = setupFixture();
  let remoteCalls = 0,
    localCalls = 0,
    completions = 0;
  const originalRemote = (r.peer.setRemoteDescription = async () => {
    remoteCalls++;
    r.transceivers[0].currentDirection = "inactive";
  });
  const originalLocal = (r.peer.setLocalDescription = async () => {
    localCalls++;
  });
  try {
    const before = await holdActiveMedia(r.actor, { renderer: true });
    const remote = r.peer.setRemoteDescription({ type: "offer" });
    const local = r.peer.setLocalDescription({ type: "answer" });
    const outcomes = Promise.allSettled([remote, local]);
    remote.then(
      () => completions++,
      () => completions++,
    );
    local.then(
      () => completions++,
      () => completions++,
    );
    r.peer.close();
    r.track.stop();
    await new Promise((resolve) => setTimeout(resolve, 30));
    assert.equal(completions, 0);
    assert.equal(remoteCalls, 0);
    assert.equal(localCalls, 0);
    assert.equal(r.transceivers[0].currentDirection, "recvonly");
    assert.equal(r.peer.connectionState, "connected");
    assert.equal(r.track.readyState, "live");
    const after = await heldMedia(r.actor);
    assert.ok(after.frames > before.frames);
    assert.equal(after.counters[0].binding.sameNativeTransceiver, true);
    assert.throws(() => revocationStop(before, after), CheckFailure);
    assert.deepEqual(
      JSON.parse(JSON.stringify(after.heldNativeNegotiationRequests)),
      { setRemoteDescription: 1, setLocalDescription: 1 },
    );
    await releaseHeld(r.actor);
    const released = await outcomes;
    assert.ok(
      released.every(
        (result) =>
          result.status === "rejected" &&
          result.reason.message === "E2E_HELD_NATIVE_NEGOTIATION_RELEASED",
      ),
    );
    assert.equal(r.peer.setRemoteDescription, originalRemote);
    assert.equal(r.peer.setLocalDescription, originalLocal);
    assert.equal(r.peer.connectionState, "closed");
    assert.equal(r.track.readyState, "ended");
  } finally {
    await releaseHeld(r.actor);
  }
});

test("native negotiation waits have one shared finite bound and are all rejected even if another cleanup fails", async () => {
  const r = setupFixture();
  const originalRemote = r.peer.setRemoteDescription,
    originalLocal = r.peer.setLocalDescription;
  try {
    await holdActiveMedia(r.actor, { renderer: true });
    const pending = Array.from({ length: 64 }, (_, i) =>
      i % 2 === 0
        ? r.peer.setRemoteDescription({ type: "offer" })
        : r.peer.setLocalDescription({ type: "answer" }),
    );
    const outcomes = Promise.allSettled(pending);
    await assert.rejects(
      r.peer.setRemoteDescription({ type: "offer" }),
      /E2E_HELD_NATIVE_NEGOTIATION_OVERFLOW/,
    );
    assert.deepEqual(
      JSON.parse(JSON.stringify(r.state.heldNativeNegotiationRequests)),
      { setRemoteDescription: 32, setLocalDescription: 32 },
    );
    r.state.restoreClose.unshift(() => {
      throw new Error("PRIVATE earlier native cleanup failed");
    });
    await assert.rejects(releaseHeld(r.actor), /held-renderer-cleanup-failed/);
    assert.ok((await outcomes).every((result) => result.status === "rejected"));
    assert.equal(r.state.heldNativeNegotiationWaits.length, 0);
    assert.equal(r.peer.setRemoteDescription, originalRemote);
    assert.equal(r.peer.setLocalDescription, originalLocal);
    assert.equal(r.peer.connectionState, "closed");
    r.state.restoreClose.shift();
  } finally {
    await releaseHeld(r.actor);
  }
});

test("inactive native Consumer transceiver cannot establish a held-renderer positive control", async () => {
  const r = setupFixture();
  r.transceivers[0].currentDirection = "inactive";
  try {
    await assert.rejects(
      holdActiveMedia(r.actor, { renderer: true }),
      /E2E_HELD_CONSUMER_IDENTITY_UNAVAILABLE/,
    );
    assert.equal(r.plays(), 0);
  } finally {
    await releaseHeld(r.actor);
  }
});

test("held renderer plays only the actual bound Consumer; the native SDK probator remains held without play", async () => {
  const probator = unmappedReceiver();
  const r = setupFixture({ extraReceiver: probator });
  try {
    const baseline = await deadlineProbe(
      () => holdActiveMedia(r.actor, { renderer: true }),
      Date.now() + 1_000,
    );
    assert.ok(baseline.frames > 1);
    assert.equal(r.plays(), 1);
    assert.equal(r.extraPlays(), 0);
    assert.equal(r.state.heldRenderers.length, 1);
    assert.equal(r.state.heldRenderers[0].track, r.track);
    assert.equal(probator.track.readyState, "live");
  } finally {
    await releaseHeld(r.actor);
  }
});

for (const [name, create] of [
  ["unannounced video track", () => unmappedReceiver({ id: "foreign-video" })],
  [
    "foreign SSRC",
    () => {
      const r = unmappedReceiver();
      r.getStats = async () =>
        new Map([
          [
            "foreign",
            {
              type: "inbound-rtp",
              kind: "video",
              ssrc: 999,
              packetsReceived: 1,
              bytesReceived: 50,
              framesDecoded: 0,
            },
          ],
        ]);
      return r;
    },
  ],
  ...["packetsReceived", "bytesReceived", "framesDecoded"].map((key) => [
    `missing native ${key}`,
    () => {
      const r = unmappedReceiver(),
        stats = r.getStats;
      r.getStats = async () => {
        const report = await stats();
        delete [...report.values()][0][key];
        return report;
      };
      return r;
    },
  ]),
  ...["kind", "ssrc"].map((key) => [
    `missing native ${key}`,
    () => {
      const r = unmappedReceiver(),
        stats = r.getStats;
      r.getStats = async () => {
        const report = await stats();
        delete [...report.values()][0][key];
        return report;
      };
      return r;
    },
  ]),
  ...["packetsReceived", "bytesReceived"].flatMap((key) =>
    [NaN, -1].map((value) => [
      `invalid native ${key} ${value}`,
      () => {
        const r = unmappedReceiver(),
          stats = r.getStats;
        r.getStats = async () => {
          const report = await stats();
          [...report.values()][0][key] = value;
          return report;
        };
        return r;
      },
    ]),
  ),
  [
    "decoded real video masquerading as a probator",
    () => {
      const r = unmappedReceiver(),
        stats = r.getStats;
      r.getStats = async () => {
        const report = await stats();
        [...report.values()][0].framesDecoded = 1;
        return report;
      };
      return r;
    },
  ],
  [
    "two native video SSRCs",
    () => {
      const r = unmappedReceiver(),
        stats = r.getStats;
      r.getStats = async () => {
        const report = await stats();
        report.set("extra", { ...[...report.values()][0], ssrc: 444 });
        return report;
      };
      return r;
    },
  ],
])
  test(`unmapped receiver cannot be silently omitted from held rendering: ${name}`, async () => {
    const r = setupFixture({ extraReceiver: create() });
    try {
      await assert.rejects(
        holdActiveMedia(r.actor, { renderer: true }),
        /E2E_HELD_/,
      );
      assert.equal(r.plays(), 0);
      assert.equal(r.extraPlays(), 0);
    } finally {
      await releaseHeld(r.actor);
    }
  });

test("an actual native zero report identifies only one official probator without manufacturing counters", async () => {
  const probator = unmappedReceiver();
  probator.getStats = async () =>
    new Map([
      [
        "native-zero",
        {
          type: "inbound-rtp",
          kind: "video",
          ssrc: 1234,
          packetsReceived: 0,
          bytesReceived: 0,
          framesDecoded: 0,
        },
      ],
    ]);
  const r = setupFixture({ extraReceiver: probator });
  try {
    const baseline = await holdActiveMedia(r.actor, { renderer: true });
    assert.ok(baseline.frames > 1);
    assert.equal(r.extraPlays(), 0);
    assert.equal(baseline.heldProbators[0].rtpStatsAvailable, true);
    assert.equal(baseline.probatorRtpStatsUnavailable, 0);
  } finally {
    await releaseHeld(r.actor);
  }
});

test("an exact public native probator binding permits unavailable RTP stats without inventing counters or playing it", async () => {
  const probator = unmappedReceiver({ rows: [] });
  const r = setupFixture({ extraReceiver: probator });
  try {
    const baseline = await holdActiveMedia(r.actor, { renderer: true });
    assert.ok(baseline.frames > 1);
    assert.equal(r.plays(), 1);
    assert.equal(r.extraPlays(), 0);
    assert.deepEqual(JSON.parse(JSON.stringify(baseline.heldProbators)), [
      {
        mid: "probator",
        codecPayload: 127,
        mime: "video/VP8",
        rtpStatsAvailable: false,
      },
    ]);
    assert.equal(baseline.excludedSdkProbators, 1);
    assert.equal(baseline.probatorRtpStatsUnavailable, 1);
    assert.equal(baseline.counters.length, 1);
    assert.equal(
      Object.hasOwn(baseline.heldProbators[0], "packetsReceived"),
      false,
    );
    assert.equal(
      Object.hasOwn(baseline.heldProbators[0], "framesDecoded"),
      false,
    );
  } finally {
    await releaseHeld(r.actor);
  }
});

for (const [name, change] of [
  [
    "wrong native MID",
    (r) => {
      r.transceivers[1].mid = "foreign";
    },
  ],
  [
    "missing native MID",
    (r) => {
      r.transceivers[1].mid = null;
    },
  ],
  [
    "foreign native receiver",
    (r) => {
      r.transceivers[1].receiver = { track: r.transceivers[1].receiver.track };
    },
  ],
  [
    "duplicate native receiver binding",
    (r) => {
      r.transceivers.push({ ...r.transceivers[1] });
    },
  ],
  [
    "wrong payload type",
    (r) => {
      r.transceivers[1].receiver.getParameters = () => ({
        codecs: [{ mimeType: "video/VP8", payloadType: 126 }],
      });
    },
  ],
  [
    "non-video codec",
    (r) => {
      r.transceivers[1].receiver.getParameters = () => ({
        codecs: [{ mimeType: "audio/opus", payloadType: 127 }],
      });
    },
  ],
  [
    "repair codec instead of primary video",
    (r) => {
      r.transceivers[1].receiver.getParameters = () => ({
        codecs: [{ mimeType: "video/rtx", payloadType: 127 }],
      });
    },
  ],
  [
    "missing public codecs",
    (r) => {
      r.transceivers[1].receiver.getParameters = () => ({});
    },
  ],
  [
    "multiple public codecs",
    (r) => {
      r.transceivers[1].receiver.getParameters = () => ({
        codecs: [
          { mimeType: "video/VP8", payloadType: 127 },
          { mimeType: "video/VP8", payloadType: 126 },
        ],
      });
    },
  ],
  [
    "wrong track ID",
    (r) => {
      r.transceivers[1].receiver.track.id = "foreign";
    },
  ],
  [
    "source-bound reserved probe",
    (r) => {
      const receiver = r.transceivers[1].receiver;
      r.state.incomingTracks.push({
        ...consumerSource(r.peer, receiver, receiver.track),
        consumerId: "source-bound-probe",
        rtpParameters: { encodings: [{ ssrc: 555 }] },
      });
    },
  ],
  [
    "partial authenticated source binding",
    (r) => {
      const receiver = r.transceivers[1].receiver;
      r.state.incomingTracks.push({
        ...consumerSource(r.peer, receiver, receiver.track),
        track: { kind: "video" },
      });
    },
  ],
])
  test(`unavailable RTP does not exempt an unknown native video receiver: ${name}`, async () => {
    const r = setupFixture({ extraReceiver: unmappedReceiver({ rows: [] }) });
    change(r);
    try {
      await assert.rejects(
        holdActiveMedia(r.actor, { renderer: true }),
        /E2E_HELD_/,
      );
      assert.equal(r.plays(), 0);
      assert.equal(r.extraPlays(), 0);
    } finally {
      await releaseHeld(r.actor);
    }
  });

for (const [name, change] of [
  [
    "MID changed",
    (r) => {
      r.transceivers[1].mid = "foreign";
    },
  ],
  [
    "native transceiver replaced",
    (r) => {
      r.transceivers[1] = { ...r.transceivers[1] };
    },
  ],
  [
    "codec changed",
    (r) => {
      r.transceivers[1].receiver.getParameters = () => ({
        codecs: [{ mimeType: "video/VP8", payloadType: 126 }],
      });
    },
  ],
  [
    "track changed",
    (r) => {
      const receiver = r.transceivers[1].receiver;
      receiver.track = { ...receiver.track };
    },
  ],
  [
    "Consumer binding appeared",
    (r) => {
      const receiver = r.transceivers[1].receiver;
      r.state.incomingTracks.push(
        consumerSource(r.peer, receiver, receiver.track),
      );
    },
  ],
])
  test(`awaited native stats cannot stale the probator role proof: ${name}`, async () => {
    const probator = unmappedReceiver({ rows: [] });
    const r = setupFixture({ extraReceiver: probator });
    probator.getStats = async () => {
      await Promise.resolve();
      change(r);
      return new Map();
    };
    try {
      await assert.rejects(
        holdActiveMedia(r.actor, { renderer: true }),
        /E2E_HELD_PROBATOR_/,
      );
      assert.equal(r.plays(), 0);
      assert.equal(r.extraPlays(), 0);
    } finally {
      await releaseHeld(r.actor);
    }
  });

test("multiple unmapped probators and conflicting actual Consumer announcements fail acquisition", async () => {
  const r = setupFixture({ extraReceiver: unmappedReceiver({ rows: [] }) });
  const original = r.peer.getReceivers;
  const second = unmappedReceiver({ rows: [] });
  r.peer.getReceivers = () => [...original(), second];
  r.transceivers.push({ receiver: second, mid: "probator" });
  try {
    await assert.rejects(
      holdActiveMedia(r.actor, { renderer: true }),
      /E2E_HELD_UNBOUND_VIDEO_RECEIVER/,
    );
    assert.equal(r.plays(), 0);
  } finally {
    await releaseHeld(r.actor);
  }
  const duplicate = setupFixture();
  duplicate.state.incomingTracks.push({
    ...duplicate.publication,
    producerId: "conflicting-producer",
  });
  try {
    await assert.rejects(
      holdActiveMedia(duplicate.actor, { renderer: true }),
      /E2E_HELD_CONSUMER_IDENTITY_UNAVAILABLE/,
    );
    assert.equal(duplicate.plays(), 0);
  } finally {
    await releaseHeld(duplicate.actor);
  }
});

for (const key of [
  "publisher",
  "consumerId",
  "producerId",
  "epoch",
  "generation",
])
  test(`held acquisition requires the real Consumer's ${key}`, async () => {
    const r = setupFixture();
    delete r.publication[key];
    try {
      await assert.rejects(
        holdActiveMedia(r.actor, { renderer: true }),
        /E2E_HELD_CONSUMER_IDENTITY_UNAVAILABLE/,
      );
      assert.equal(r.plays(), 0);
    } finally {
      await releaseHeld(r.actor);
    }
  });

test("actual setup proves own renderer progress before deletion and late callbacks cannot restart disposed rendering", async () => {
  const r = setupFixture();
  try {
    const baseline = await holdActiveMedia(r.actor, { renderer: true });
    assert.ok(baseline.frames > 1);
    assert.equal(baseline.counterMode, "receiver-renderer");
    assert.equal(r.plays(), 1);
    await releaseHeld(r.actor);
    const item = r.state.heldRenderers[0],
      before = item.frames;
    const callbackCount = r.callbacks.size;
    for (const callback of r.callbacks.values())
      callback(0, { presentedFrames: 999 });
    assert.equal(item.frames, before);
    assert.equal(r.callbacks.size, callbackCount);
    assert.equal(r.removes(), 1);
  } finally {
    await releaseHeld(r.actor);
  }
});

test("setup play failure still has owned cleanup and stalled native callbacks never establish a positive control", async () => {
  const failed = setupFixture({ playError: true });
  await assert.rejects(
    holdActiveMedia(failed.actor, { renderer: true }),
    /PRIVATE-native-play/,
  );
  await releaseHeld(failed.actor);
  assert.equal(failed.removes(), 1);
  const stalled = setupFixture({ stalled: true });
  await assert.rejects(
    deadlineProbe(
      () => holdActiveMedia(stalled.actor, { renderer: true }),
      Date.now() + 25,
    ),
    NativeInterfaceFailure,
  );
  assert.equal(stalled.state.heldRenderers[0].frames, 0);
  await releaseHeld(stalled.actor);
  assert.equal(stalled.removes(), 1);
});

test("partial DOM acquisition remains owned and failed native callback rescheduling cannot freeze into a stop pass", async () => {
  const append = setupFixture({ appendError: true });
  await assert.rejects(
    holdActiveMedia(append.actor, { renderer: true }),
    /PRIVATE-append/,
  );
  await releaseHeld(append.actor);
  assert.equal(append.removes(), 1);
  assert.equal(append.plays(), 0);
  const callback = setupFixture({ rescheduleError: true });
  await assert.rejects(
    holdActiveMedia(callback.actor, { renderer: true }),
    CheckFailure,
  );
  assert.equal(callback.state.heldRenderers[0].counterFailed, true);
  await releaseHeld(callback.actor);
  assert.equal(callback.removes(), 1);
});
