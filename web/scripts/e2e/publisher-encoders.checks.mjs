import { test } from "node:test";
import { setTimeout, clearTimeout } from "node:timers";
import assert from "node:assert/strict";
import { createContext, runInContext } from "node:vm";
import { publisherEncoderProgress } from "./publisher-encoders.mjs";
import { sampleVideoSenders } from "./probe.mjs";

function observed(layers = 1, frames = 10) {
  return {
    captures: [
      { capture: 0, kind: "display", slot: 1 },
      { capture: 1, kind: "camera", slot: 1 },
      { capture: 2, kind: "display", slot: 2 },
    ],
    senders: [0, 1, 2].map((capture) => ({
      peer: 0,
      sender: capture,
      capture,
      current: true,
      encodings: Array.from({ length: layers }, (_, index) => ({
        rid: layers > 1 ? ["q", "f"][index] : null,
        frames,
        packets: frames,
      })),
    })),
  };
}

for (const layers of [1, 2])
  test(`${layers} native encodings per each of three sources require their own fresh progress`, () => {
    const before = observed(layers);
    assert.equal(publisherEncoderProgress(before), true);
    assert.equal(publisherEncoderProgress(observed(layers, 11), before), true);
    assert.equal(publisherEncoderProgress(before, before), false);
    const partial = observed(layers, 11);
    partial.senders[1].encodings = before.senders[1].encodings;
    assert.equal(publisherEncoderProgress(partial, before), false);
  });

test("six advancing encodings on one actual source cannot replace three capture tracks", () => {
  const only = observed(2);
  only.captures = only.captures.slice(0, 1);
  only.senders = only.senders.slice(0, 1);
  only.senders[0].encodings = Array.from({ length: 6 }, () => ({ frames: 99 }));
  assert.equal(publisherEncoderProgress(only), false);
  const duplicate = observed(2);
  duplicate.senders[2].capture = 0;
  assert.equal(publisherEncoderProgress(duplicate), false);
});

test("missing source, unknown sender, changed peer and retired track cannot satisfy readiness", () => {
  const before = observed(2);
  for (const mutate of [
    (next) => {
      next.senders[1].encodings = [];
    },
    (next) => {
      next.senders[1].current = false;
    },
    (next) => {
      next.senders[1].capture = null;
    },
    (next) => {
      next.senders[1].peer = 1;
    },
    (next) => {
      next.senders[1].sender = 5;
    },
    (next) => {
      next.captures[1].kind = "display";
    },
    (next) => {
      next.senders.push({ ...next.senders[0] });
    },
  ]) {
    const next = observed(2, 11);
    mutate(next);
    assert.equal(publisherEncoderProgress(next, before), false);
  }
});

function nativeFixture({
  closed = false,
  replacement = false,
  foreign = false,
  failure = false,
  pending = false,
} = {}) {
  let calls = 0;
  const track = { id: "private-track-id", kind: "video", readyState: "live" };
  const sender = {
    track,
    getStats: async () => {
      calls++;
      if (pending) return new Promise(() => {});
      if (failure) throw new Error("native sender failure");
      if (replacement) sender.track = { ...track, id: "new-private-id" };
      return new Map([
        [
          "source",
          {
            type: "media-source",
            trackIdentifier: foreign ? "other-private-id" : track.id,
          },
        ],
        ...["q", "f"].map((rid) => [
          rid,
          {
            type: "outbound-rtp",
            kind: "video",
            rid,
            mediaSourceId: "source",
            framesEncoded: 12,
            packetsSent: 34,
          },
        ]),
        ["audio", { type: "outbound-rtp", kind: "audio", framesEncoded: 999 }],
      ]);
    },
  };
  const context = createContext({
    window: {
      __e2e: {
        captures: [{ kind: "display", slot: 1, track }],
        peers: [
          {
            connectionState: closed ? "closed" : "connected",
            getSenders: () => [sender],
          },
        ],
      },
    },
    setTimeout,
    clearTimeout,
  });
  return {
    read: (deadlineEpochMs) =>
      runInContext(
        `(${sampleVideoSenders.toString()})(${JSON.stringify({ deadlineEpochMs })})`,
        context,
      ),
    calls: () => calls,
  };
}

test("native sender selection preserves capture binding and both RIDs while redacting raw identifiers", async () => {
  const fixture = nativeFixture();
  const result = await fixture.read();
  assert.equal(fixture.calls(), 1);
  assert.deepEqual(JSON.parse(JSON.stringify(result)), {
    captures: [{ capture: 0, kind: "display", slot: 1 }],
    senders: [
      {
        peer: 0,
        sender: 0,
        capture: 0,
        current: true,
        encodings: [
          { rid: "q", frames: 12, packets: 34 },
          { rid: "f", frames: 12, packets: 34 },
        ],
      },
    ],
  });
  assert.equal(JSON.stringify(result).includes("private"), false);
});

test("native own-track mismatch and asynchronous sender replacement cannot train the readiness control", async () => {
  const foreign = await nativeFixture({ foreign: true }).read();
  assert.equal(foreign.senders[0].encodings.length, 0);
  const retired = await nativeFixture({ replacement: true }).read();
  assert.equal(retired.senders[0].current, false);
});

test("closed peers are not probed and native failures or never-resolving stats remain red", async () => {
  const closed = nativeFixture({ closed: true });
  assert.equal((await closed.read()).senders.length, 0);
  assert.equal(closed.calls(), 0);
  await assert.rejects(
    nativeFixture({ failure: true }).read(),
    /native sender failure/,
  );
  await assert.rejects(
    nativeFixture({ pending: true }).read(Date.now() + 25),
    /E2E_NATIVE_STATS_DEADLINE/,
  );
});
