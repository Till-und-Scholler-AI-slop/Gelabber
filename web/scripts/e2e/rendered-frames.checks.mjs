import { test } from "node:test";
import assert from "node:assert/strict";
import { createContext, runInContext } from "node:vm";
import { sample } from "./probe.mjs";

function probe({ quality, callbacks = true, peers = [] }) {
  let next;
  const video = {
    readyState: 0,
    videoWidth: 0,
    videoHeight: 0,
    paused: false,
    closest: () => ({
      querySelector: () => ({ textContent: "fixture — Live" }),
    }),
    getVideoPlaybackQuality: () => ({ totalVideoFrames: quality }),
    ...(callbacks
      ? {
          requestVideoFrameCallback: (fn) => {
            next = fn;
          },
        }
      : {}),
  };
  const context = createContext({
    window: {
      __e2e: {
        peers,
        sockets: [],
        captures: [],
        renderedVideos: new WeakMap(),
      },
    },
    document: { querySelectorAll: () => [video] },
  });
  return {
    read: async () =>
      (await runInContext(`(${sample.toString()})()`, context)).videos[0],
    present: (frames) => {
      assert.ok(next);
      next(1, { presentedFrames: frames });
    },
  };
}
test("zero playback quality requires real native frame callbacks; sampling alone cannot invent progress", async () => {
  const p = probe({ quality: 0 });
  const first = await p.read();
  assert.equal(first.renderedFrames, 0);
  assert.equal((await p.read()).renderedFrames, 0);
  p.present(12);
  const next = await p.read();
  assert.equal(next.renderedFrames, 12);
  assert.equal(next.renderedFramesSource, "native-video-frame-callback");
  p.present(14);
  const last = await p.read();
  assert.ok(last.renderedFrames > next.renderedFrames);
  assert.equal(last.videoFrameCallbacks, 2);
  assert.equal(last.playbackQualityFrames, 0);
});
test("positive playback-quality count keeps its native source while unavailable counters never progress", async () => {
  const native = await probe({ quality: 19 }).read();
  assert.equal(native.renderedFrames, 19);
  assert.equal(native.renderedFramesSource, "native-playback-quality");
  const absent = probe({ quality: undefined, callbacks: false });
  assert.equal((await absent.read()).renderedFrames, 0);
  assert.equal((await absent.read()).renderedFramesSource, "unavailable");
  assert.equal((await absent.read()).renderedFrames, 0);
});

test("closed peers never call rejected native getStats; live-peer stats errors remain failures", async () => {
  let calls = 0;
  const peer = {
    connectionState: "closed",
    iceConnectionState: "closed",
    getStats: async () => {
      calls++;
      throw new Error("native closed peer");
    },
    get localDescription() {
      if (this.connectionState === "closed")
        throw new Error("native closed getter");
      return null;
    },
    getTransceivers: () => [],
    getSenders: () => [],
    getReceivers: () => [],
  };
  await probe({ quality: 1, peers: [peer] }).read();
  assert.equal(calls, 0);
  peer.connectionState = "connected";
  await assert.rejects(probe({ quality: 1, peers: [peer] }).read());
  assert.equal(calls, 1);
  peer.getStats = async () => {
    calls++;
    peer.connectionState = "closed";
    throw new Error("native close race");
  };
  await probe({ quality: 1, peers: [peer] }).read();
  assert.equal(calls, 2);
});
