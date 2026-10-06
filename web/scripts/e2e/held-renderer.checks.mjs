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
  const pc = { getReceivers: () => [receiver] };
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
  return { actor, state, item, cleanup, statsCalls: () => statsCalls };
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
    getReceivers: () => [receiver, ...(extraReceiver ? [extraReceiver] : [])],
    getTransceivers: () => transceivers,
  };
  const transceivers = [
    { receiver, mid: "consumer-0" },
    ...(extraReceiver ? [{ receiver: extraReceiver, mid: "probator" }] : []),
  ];
  const publication = consumerSource(peer, receiver, track);
  const state = {
    peers: [peer],
    sockets: [],
    incomingTracks: [publication],
  };
  const context = vm.createContext({
    window: { __e2e: state, RTCPeerConnection: { prototype: { close() {} } } },
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
