/* global URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import { CheckFailure } from "./harness.mjs";
import { sample, voiceOnlyControl } from "./voice-only-control.mjs";

test("source identity reorder delivers authoritative Consumer messages without native track events", async () => {
  const delivered = [];
  const state = {
    holdTracks: false,
    heldTracks: ["camera", "live"].map((source) => ({
      event: Object.freeze({
        data: JSON.stringify({ op: "consumer", source }),
      }),
      kind: "video",
      deliver: () => delivered.push(source),
    })),
  };
  const element = {
    filter() {
      return this;
    },
    getByRole() {
      return this;
    },
    first() {
      return this;
    },
    async waitFor() {},
    async count() {
      return 1;
    },
    async click() {},
  };
  const actor = { page: { locator: () => element } };
  const mocks = {
    "./harness.mjs": {
      CheckFailure,
      check: (ok, code) => assert.ok(ok, code),
      click: async () => {},
      liveTiles: (page) => page.locator("figure"),
      snapshot: async () => ({ heldVideoTracks: state.heldTracks.length }),
      observe: async () => {},
      until: async (read, accepts) => {
        const value = await read(5_000);
        assert.ok(accepts(value));
        return value;
      },
    },
    "./media.mjs": {
      watchSource: async () => {},
      activePeers: () => [],
      armPlaybackRetry: () => {},
      progress: async (_actor, options) => ({ kind: options.kind ?? "live" }),
    },
    "./native-evaluate.mjs": {
      nativeEvaluate: async (_actor, fn, arg) => fn(arg),
      deadlineProbe: async (fn) => fn(),
      NativeInterfaceFailure: class extends Error {},
    },
    "./probe.mjs": {
      sampleVideoSenders: async () => ({ actualFixture: true }),
      samplePublicationFlow: async () => ({}),
    },
    "./publisher-encoders.mjs": {
      publisherEncoderProgress: (value) => value.actualFixture === true,
    },
  };
  const context = vm.createContext({ window: { __e2e: state }, Date });
  const source = new vm.SourceTextModule(
    await readFile(new URL("./media-extra.mjs", import.meta.url), "utf8"),
    { context },
  );
  await source.link(async (name) => {
    const exports = mocks[name] ?? (await import(name));
    return new vm.SyntheticModule(
      Object.keys(exports),
      function () {
        for (const [key, value] of Object.entries(exports))
          this.setExport(key, value);
      },
      { context },
    );
  });
  await source.evaluate();
  let result;
  await source.namespace.mediaExtraScenarios(
    {
      run: async (id, _predecessors, task) => {
        if (id === "forced-track-arrival-reorder-source-identity")
          result = await task();
      },
    },
    { owner: actor, member: actor, watcher: actor },
    { begin: async () => {}, reset: async () => {}, options: {} },
  );
  assert.deepEqual(delivered, ["live", "camera"]);
  assert.deepEqual(Array.from(result.deliveredKinds), ["video", "video"]);
  assert.equal(result.consumerAnnouncementOrder, "reversed");
  assert.equal(state.holdTracks, false);
  assert.equal(state.heldTracks.length, 0);
});

test("exact SDP scenario preserves failed bounded camera warmup and never injects SDP or claims progress afterward", async () => {
  let clock = 1_000,
    injected = 0,
    progressCalls = 0,
    failure;
  const actor = {
    page: {
      evaluate: async () => {
        injected++;
      },
    },
  };
  const mocks = {
    "./harness.mjs": {
      CheckFailure,
      check() {},
      click: async () => {},
      liveTiles: (page) => page.locator("figure"),
      snapshot: async () => ({}),
      observe: async () => {},
      until: async (_probe, _accept, code, deadline) => {
        assert.equal(code, "fixture-sdp-camera-warmup-deadline");
        assert.equal(deadline, 20_000);
        clock += deadline;
        throw new CheckFailure(code, {
          last: { videos: [{ kind: "camera", width: 480, height: 270 }] },
        });
      },
    },
    "./media.mjs": {
      watchSource: async () => {},
      activePeers: () => [],
      armPlaybackRetry: () => {},
      progress: async () => {
        progressCalls++;
      },
    },
  };
  const context = vm.createContext({
    Date: class {
      static now() {
        return clock;
      }
    },
  });
  const source = new vm.SourceTextModule(
    await readFile(new URL("./media-extra.mjs", import.meta.url), "utf8"),
    { context },
  );
  await source.link(async (name) => {
    const exports = mocks[name] ?? (await import(name));
    return new vm.SyntheticModule(
      Object.keys(exports),
      function () {
        for (const [key, value] of Object.entries(exports))
          this.setExport(key, value);
      },
      { context },
    );
  });
  await source.evaluate();
  const h = {
    run: async (id, _predecessors, task) => {
      if (id !== "rejected-native-sdp-keeps-voice-other-source-no-ghost")
        return;
      try {
        await task();
      } catch (error) {
        failure = error;
      }
    },
  };
  await source.namespace.mediaExtraScenarios(
    h,
    { owner: actor, member: actor },
    {
      begin: async () => {},
      reset: async () => {},
      options: { budget: 5_000 },
    },
  );
  assert.ok(failure instanceof CheckFailure);
  assert.equal(failure.metrics.faultExercised, false);
  assert.deepEqual(
    JSON.parse(JSON.stringify(failure.metrics.cameraFixtureWarmup)),
    {
      deadlineMs: 20_000,
      status: "FAIL",
      WarmupMs: 20_000,
      width: 480,
      height: 270,
    },
  );
  assert.equal(progressCalls, 0);
  assert.equal(injected, 0);
});

import { sessionAudioControl } from "./session-audio-control.mjs";
import * as mediaExtra from "./media-extra.mjs";
import { NativeInterfaceFailure } from "./native-evaluate.mjs";
test("actual audio microphone fixture waits for the freshly joined connected sender and its own RTP", async () => {
  const r = await sessionAudioControl({ micReadyAfterPoll: true });
  assert.equal(r.error, undefined);
  assert.equal(
    r.actions.filter((a) => a === "microphone-fixture-poll").length,
    2,
  );
  assert.equal(r.result.microphoneControl.senderCount, 1);
  assert.ok(r.result.microphoneControl.outboundAudioPackets > 0);
});
for (const option of [
  { micNeverConnected: true },
  { micNoOutboundRtp: true },
  { otherPeerOnlyRtp: true },
  { samePeerOtherSenderOnlyRtp: true },
]) {
  test(`actual microphone fixture cannot accept missing connection/RTP or another peer's RTP ${JSON.stringify(option)}`, async () => {
    const r = await sessionAudioControl(option);
    assert.ok(r.error instanceof CheckFailure);
    assert.equal(
      r.error.message,
      "fixture-session-positive-microphone-not-ready",
    );
    assert.equal(r.result, undefined);
    assert.equal(r.watcherState.rejectPlayback, false);
    if (option.samePeerOtherSenderOnlyRtp) {
      assert.ok(r.watcherSenderStatsCalls > 0);
      assert.equal(r.error.metrics.last.outboundAudioPackets, 0);
      assert.ok(r.error.metrics.last.senderCount === 1);
      assert.ok(r.error.metrics.last.peerIndex !== null);
    }
  });
}
test("actual audio task retries playback with the real click, then keeps the microphone across mute/deafen/undeafen", async () => {
  const r = await sessionAudioControl();
  assert.equal(r.error, undefined);
  assert.equal(r.watcherState.retryArmed, true);
  assert.equal(r.result.playing.retryShown, 0);
  assert.equal(r.result.muted.peers[0].audioSenders.length, 1);
  assert.equal(r.result.deafened.peers[0].audioSenders.length, 1);
  assert.equal(r.result.hearing.peers[0].audioSenders.length, 1);
  assert.equal(r.watcherState.rejectPlayback, false);
});
for (const [option, code] of [
  [{ retryStartsNothing: true }, "audio-click-retry-did-not-play-every-path"],
  [{ noticeStays: true }, "audio-click-retry-did-not-play-every-path"],
  [{ muteDoesNothing: true }, "session-mute-did-not-disable-mic"],
  [{ deafenSparesOne: true }, "session-deafen-did-not-mute-every-path"],
  [{ undeafenSparesOne: true }, "session-undeafen-did-not-restore-every-path"],
  [{ volumeSparesOne: true }, "session-volume-not-applied-to-every-path"],
]) {
  test(`actual audio task fails when the app breaks it ${JSON.stringify(option)}`, async () => {
    const r = await sessionAudioControl(option);
    assert.ok(r.error instanceof CheckFailure);
    assert.equal(r.error.message, code);
    assert.equal(r.watcherState.rejectPlayback, false);
  });
}
for (const option of [
  { lostMicAt: "deafen" },
  { lostMicAt: "undeafen" },
  { replaceSender: true },
  { replaceTrack: true },
]) {
  test(`actual audio task rejects microphone loss/replacement ${JSON.stringify(option)}`, async () => {
    const r = await sessionAudioControl(option);
    assert.ok(r.error instanceof CheckFailure);
    assert.equal(r.watcherState.rejectPlayback, false);
  });
}
test("actual audio task restore rejection is a visible redacted failure", async () => {
  const r = await sessionAudioControl({ finalRestoreError: true });
  assert.ok(r.actions.includes("final-restore-rejected"));
  assert.ok(r.error instanceof NativeInterfaceFailure);
  assert.equal(r.error.metrics.stage, "session-audio-owned-restore");
  assert.deepEqual(r.error.metrics.failedSteps, ["playback-restore"]);
  assert.ok(!JSON.stringify(r.error).includes("PRIVATE"));
});
test("actual audio restore never evaluates a quarantined page", async () => {
  let evaluates = 0;
  const watcher = {
    nativeEvaluationUnusable: true,
    page: {
      evaluate: () => {
        evaluates++;
      },
    },
  };
  await mediaExtra.restoreSessionPlayback(watcher, 5);
  assert.equal(evaluates, 0);
});
test("actual audio restore bounds a pending evaluation and quarantines the page once", async () => {
  let abortCloses = 0;
  const watcher = {
    page: {
      evaluate: () => new Promise(() => {}),
      close: async () => {
        abortCloses++;
      },
      isClosed: () => false,
    },
  };
  await assert.rejects(
    mediaExtra.restoreSessionPlayback(watcher, 5),
    (error) =>
      error instanceof NativeInterfaceFailure &&
      error.metrics.failedSteps.join(",") === "playback-restore",
  );
  await watcher.nativeAbortClose;
  assert.equal(watcher.nativeEvaluationUnusable, true);
  assert.equal(abortCloses, 1);
});

// The scenario takes four samples per side: before, progress, attached,
// judged (the audio moved on past the attached one).
const four = (late = {}) => [
  sample({ packets: 10 }),
  sample({ packets: 20, ...late }),
  sample({ packets: 20, ...late }),
  sample({ packets: 30, ...late }),
];
const member = four();
test("voice-only waits for the owner's playback that attaches after its packets", async () => {
  const r = await voiceOnlyControl({
    owner: [
      sample({ packets: 10 }),
      // The packets grew, the playback is not attached yet: one sample later.
      sample({ packets: 20, plays: 0 }),
      sample({ packets: 20 }),
      sample({ packets: 30 }),
    ],
    member,
  });
  assert.equal(r.error, undefined);
  assert.deepEqual(r.result.judged[0].roomAudio.perSource, [1]);
});
for (const [name, owner, code] of [
  [
    "a second path for the source",
    four({ plays: 2 }),
    "voice-only-source-duplicate-or-unexpected-capture",
  ],
  [
    "a second path that comes after the first",
    [
      sample({ packets: 10 }),
      sample({ packets: 20 }),
      // Attached with one path; the second comes as the audio moves on.
      sample({ packets: 20 }),
      sample({ packets: 30, plays: 2 }),
    ],
    "voice-only-source-duplicate-or-unexpected-capture",
  ],
  [
    "two sources",
    four({ perSource: [1, 1] }),
    "voice-only-source-duplicate-or-unexpected-capture",
  ],
  [
    "a foreign source",
    four({ foreign: 1 }),
    "voice-only-source-duplicate-or-unexpected-capture",
  ],
  [
    "no playback ever",
    four({ plays: 0 }),
    "voice-only-duplex-playback-not-attached",
  ],
])
  test(`voice-only fails on ${name}`, async () => {
    const r = await voiceOnlyControl({ owner, member });
    assert.ok(r.error instanceof CheckFailure, String(r.error));
    assert.equal(r.error.message, code);
  });

// GEL-460: a passing retry keeps its step times, so a slow one that still
// passes says which step took its time.
async function sdpRetryRow(firstFrameMs, { evidenceFails = false } = {}) {
  const clock = { now: 10_000 };
  const flows = {
    owner: {
      rpc: [
        { method: "produce", kind: "v", sentAt: 1, answeredAt: 2 },
        { method: "produce", kind: "s", sentAt: 10_400, answeredAt: 10_450 },
      ],
      consumers: [],
    },
    member: {
      rpc: [
        { method: "w", kind: "s", sentAt: 10_900, answeredAt: 10_920 },
        {
          method: "consumerReady",
          kind: "s",
          sentAt: 11_000,
          answeredAt: 11_010,
        },
      ],
      consumers: [
        { arrivedAt: 10_990, layers: { spatial: 0, temporal: 2, at: 11_200 } },
      ],
    },
  };
  const actor = (name) => ({
    page: {
      evaluate: async (fn) => {
        if (fn.name !== "samplePublicationFlow") return undefined;
        if (evidenceFails) throw new Error("evidence-read-failed");
        return flows[name];
      },
    },
  });
  const snapshot = {
    videos: [{ kind: "camera", width: 320, height: 180 }],
    peers: [],
    rejectedSdp: 1,
    displayCalls: 1,
    captures: [{ kind: "display", slot: 1, state: "ended" }],
  };
  const mocks = {
    "./harness.mjs": {
      CheckFailure,
      check() {},
      click: async () => {},
      liveTiles: (page) => page.locator("figure"),
      snapshot: async () => snapshot,
      observe: async () => {},
      until: async () => snapshot,
    },
    "./media.mjs": {
      watchSource: async () => {},
      activePeers: () => [],
      armPlaybackRetry: () => {},
      // The green retry: its wait began at the click.
      progress: async (_actor, { color }) =>
        color[1] === 220
          ? {
              firstFrameMs,
              startedAt: clock.now,
              watchedAt: clock.now + 600,
              last: snapshot,
            }
          : { last: snapshot },
    },
  };
  const context = vm.createContext({
    Date: class {
      static now() {
        return clock.now;
      }
    },
  });
  const source = new vm.SourceTextModule(
    await readFile(new URL("./media-extra.mjs", import.meta.url), "utf8"),
    { context },
  );
  await source.link(async (name) => {
    const exports = mocks[name] ?? (await import(name));
    return new vm.SyntheticModule(
      Object.keys(exports),
      function () {
        for (const [key, value] of Object.entries(exports))
          this.setExport(key, value);
      },
      { context },
    );
  });
  await source.evaluate();
  let row;
  const h = {
    run: async (id, _predecessors, task) => {
      if (id === "rejected-native-sdp-keeps-voice-other-source-no-ghost")
        row = await task();
    },
  };
  await source.namespace.mediaExtraScenarios(
    h,
    { owner: actor("owner"), member: actor("member") },
    {
      begin: async () => {},
      reset: async () => {},
      options: { budget: 5_000 },
    },
  );
  return JSON.parse(JSON.stringify(row));
}
test("a passing Live retry keeps its step times, and a slow one its raw records too", async () => {
  const fast = await sdpRetryRow(1_200);
  assert.deepEqual(fast.retrySteps, {
    produceSent: 400,
    produceAnswered: 450,
    watchSent: 900,
    watchAnswered: 920,
    consumerArrived: 990,
    consumerReadySent: 1_000,
    consumerReadyAnswered: 1_010,
    lastLayerReport: 1_200,
    layer: { spatial: 0, temporal: 2 },
    firstFrame: 1_200,
  });
  assert.equal(fast.publisherLive, undefined);
  const slow = await sdpRetryRow(4_465);
  assert.equal(slow.retrySteps.firstFrame, 4_465);
  assert.equal(slow.publisherLive.rpc.length, 2);
  assert.equal(slow.subscriberLive.consumers.length, 1);
});
test("a Live retry whose evidence cannot be read still passes, with null steps", async () => {
  const row = await sdpRetryRow(4_465, { evidenceFails: true });
  assert.equal(row.retrySteps.produceSent, null);
  assert.equal(row.retrySteps.consumerArrived, null);
  assert.equal(row.retrySteps.firstFrame, 4_465);
  assert.equal(row.publisherLive, null);
});
