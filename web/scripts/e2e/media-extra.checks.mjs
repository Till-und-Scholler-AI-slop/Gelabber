/* global URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import { CheckFailure } from "./harness.mjs";

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
      activePeers: () => [],
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
    assert.ok(r.actions.includes("other.context.close"));
    if (option.samePeerOtherSenderOnlyRtp) {
      assert.ok(r.watcherSenderStatsCalls > 0);
      assert.equal(r.error.metrics.last.outboundAudioPackets, 0);
      assert.ok(r.error.metrics.last.senderCount === 1);
      assert.ok(r.error.metrics.last.peerIndex !== null);
    }
  });
}
test("actual audio task retains the original expected microphone across mute/deafen/undeafen", async () => {
  const r = await sessionAudioControl();
  assert.equal(r.error, undefined);
  assert.equal(r.result.muted.peers[0].audioSenders.length, 1);
  assert.equal(r.result.deafened.peers[0].audioSenders.length, 1);
  assert.equal(r.result.hearing.peers[0].audioSenders.length, 1);
  assert.ok(r.actions.includes("other.context.close"));
});
for (const option of [
  { lostMicAt: "deafen" },
  { lostMicAt: "undeafen" },
  { replaceSender: true },
  { replaceTrack: true },
]) {
  test(`actual audio task rejects microphone loss/replacement ${JSON.stringify(option)}`, async () => {
    const r = await sessionAudioControl(option);
    assert.ok(r.error instanceof CheckFailure);
    assert.ok(r.actions.includes("other.context.close"));
  });
}

import * as mediaExtra from "./media-extra.mjs";
import { NativeInterfaceFailure } from "./native-evaluate.mjs";
test("actual audio task restore rejection still closes other context with visible redacted failure", async () => {
  const r = await sessionAudioControl({ finalRestoreError: true });
  assert.ok(r.error);
  assert.ok(r.actions.includes("final-restore-rejected"));
  assert.ok(r.actions.includes("other.context.close"));
  assert.ok(r.error instanceof NativeInterfaceFailure);
  assert.equal(r.error.metrics.stage, "session-audio-owned-restore");
  assert.deepEqual(r.error.metrics.failedSteps, ["playback-restore"]);
  assert.ok(!JSON.stringify(r.error).includes("PRIVATE"));
});
test("actual audio task attempts both failed restore and failed independent close", async () => {
  const r = await sessionAudioControl({
    finalRestoreError: true,
    otherCloseError: true,
  });
  assert.ok(r.actions.includes("other.context.close"));
  assert.deepEqual(r.error.metrics.failedSteps, [
    "playback-restore",
    "other-context-close",
  ]);
});
test("actual audio task independent close rejection remains visible after healthy restore", async () => {
  const r = await sessionAudioControl({ otherCloseError: true });
  assert.ok(r.error instanceof NativeInterfaceFailure);
  assert.deepEqual(r.error.metrics.failedSteps, ["other-context-close"]);
  assert.ok(!JSON.stringify(r.error).includes("PRIVATE"));
});
test("actual audio finally never evaluates quarantined page and bounds pending independent context close", async () => {
  let evaluates = 0,
    closes = 0;
  const watcher = {
    nativeEvaluationUnusable: true,
    page: {
      evaluate: () => {
        evaluates++;
      },
    },
  };
  const other = {
    context: {
      close: () => {
        closes++;
        return new Promise(() => {});
      },
    },
  };
  assert.equal(typeof mediaExtra.closeSessionAudioActors, "function");
  await assert.rejects(
    mediaExtra.closeSessionAudioActors(watcher, other, 5),
    (error) =>
      error instanceof NativeInterfaceFailure &&
      error.metrics.failedSteps.join(",") === "other-context-close",
  );
  assert.equal(evaluates, 0);
  assert.equal(closes, 1);
});

test("actual audio finally bounds a pending restore, quarantines once and still attempts independent context close", async () => {
  let abortCloses = 0,
    otherCloses = 0;
  const watcher = {
    page: {
      evaluate: () => new Promise(() => {}),
      close: async () => {
        abortCloses++;
      },
      isClosed: () => false,
    },
  };
  const other = {
    context: {
      close: async () => {
        otherCloses++;
      },
    },
  };
  assert.equal(typeof mediaExtra.closeSessionAudioActors, "function");
  await assert.rejects(
    mediaExtra.closeSessionAudioActors(watcher, other, 5),
    (error) =>
      error instanceof NativeInterfaceFailure &&
      error.metrics.failedSteps.join(",") === "playback-restore",
  );
  await watcher.nativeAbortClose;
  assert.equal(watcher.nativeEvaluationUnusable, true);
  assert.equal(abortCloses, 1);
  assert.equal(otherCloses, 1);
});
