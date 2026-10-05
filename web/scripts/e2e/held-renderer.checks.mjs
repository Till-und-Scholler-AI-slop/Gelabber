import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { setTimeout, clearTimeout } from "node:timers";
import { URL } from "node:url";
import { readFile } from "node:fs/promises";
import { heldMedia, holdActiveMedia, releaseHeld } from "./access.mjs";
import { deadlineProbe, NativeInterfaceFailure } from "./native-evaluate.mjs";
import { check, CheckFailure } from "./harness.mjs";

function fixture({ source = "native-video-frame-callback", frames = 20 } = {}) {
  const track = { kind: "video", readyState: "live", enabled: true };
  const receiver = { track };
  const stream = { getVideoTracks: () => [track] };
  const cleanup = [];
  const item = {
    source,
    receiver,
    track,
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
  const state = {
    heldCounterMode: "receiver-renderer",
    heldRenderers: [item],
    heldExpectedRenderers: [
      {
        renderer: item,
        source,
        video: item.video,
        receiver,
        track,
        callbackFunction: item.callbackFunction,
      },
    ],
    heldPeers: [
      {
        getStats: async () => {
          statsCalls++;
          return new Map();
        },
      },
    ],
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
} = {}) {
  const callbacks = new Map(),
    timers = new Map();
  let sequence = 0,
    presented = 0,
    removes = 0,
    plays = 0;
  const track = { kind: "video", readyState: "live", enabled: true };
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
    getReceivers: () => [receiver],
  };
  const state = { peers: [peer], sockets: [] };
  const context = vm.createContext({
    window: { __e2e: state, RTCPeerConnection: { prototype: { close() {} } } },
    document: {
      createElement: () => video,
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
  };
}

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
