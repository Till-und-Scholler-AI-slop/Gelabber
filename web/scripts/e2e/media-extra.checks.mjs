/* global URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import { CheckFailure } from "./harness.mjs";

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
