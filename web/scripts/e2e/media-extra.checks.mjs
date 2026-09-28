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
