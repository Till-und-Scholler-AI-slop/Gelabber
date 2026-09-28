/* global setTimeout, clearTimeout */
import { test } from "node:test";
import assert from "node:assert/strict";
import { nativeEvaluate, NativeInterfaceFailure } from "./native-evaluate.mjs";
test("pending native evaluation is bounded, fails visibly, and never yields fabricated media data", async () => {
  const actor = { page: { evaluate: () => new Promise(() => {}) } };
  await assert.rejects(
    nativeEvaluate(actor, function sample() {}, undefined, 2),
    (e) =>
      e instanceof NativeInterfaceFailure &&
      e.metrics.testError &&
      e.metrics.nativeDataAvailable === false &&
      e.metrics.stage === "outer-page-evaluate",
  );
});
test("native success and rejection propagate unchanged and cannot be replaced with a timeout pass", async () => {
  const data = { decoded: 12, rendered: 7 };
  assert.equal(
    await nativeEvaluate({ page: { evaluate: async () => data } }, () => {}),
    data,
  );
  const error = new TypeError("native control");
  await assert.rejects(
    nativeEvaluate(
      {
        page: {
          evaluate: async () => {
            throw error;
          },
        },
      },
      () => {},
    ),
    (e) => e === error,
  );
});
import { until, observe, CheckFailure } from "./harness.mjs";
import { deadlineProbe } from "./native-evaluate.mjs";
test("actual until/observe bound never-resolving probe, quarantine it and reject late success", async () => {
  const pending = () => new Promise(() => {});
  await assert.rejects(
    until(pending, () => true, "strict-progress", 3),
    NativeInterfaceFailure,
  );
  await assert.rejects(
    observe(3, pending),
    (e) => e.metrics.stage === "probe-quarantined",
  );
  let accepted = 0;
  const late = () =>
    new Promise((resolve) => setTimeout(() => resolve({ frames: 99 }), 20));
  await assert.rejects(
    until(
      late,
      () => {
        accepted++;
        return true;
      },
      "strict-progress",
      3,
    ),
    NativeInterfaceFailure,
  );
  assert.equal(accepted, 0);
  assert.equal(
    await until(
      async () => 7,
      (n) => n === 7,
      "strict-progress",
      100,
    ),
    7,
  );
});
test("remaining parent budget reaches snapshot and timed-out page is closed once and never sampled again", async () => {
  let calls = 0,
    closes = 0;
  const actor = {
    page: {
      evaluate: () => {
        calls++;
        return new Promise(() => {});
      },
      close: async () => {
        closes++;
      },
    },
  };
  await assert.rejects(
    deadlineProbe(
      () => nativeEvaluate(actor, function sample() {}),
      Date.now() + 4,
    ),
    NativeInterfaceFailure,
  );
  await new Promise((resolve) => setTimeout(resolve, 5));
  await assert.rejects(
    nativeEvaluate(actor, function sample() {}),
    NativeInterfaceFailure,
  );
  assert.equal(calls, 1);
  assert.equal(closes, 1);
  assert.equal(actor.nativeEvaluationUnusable, true);
});
import vm from "node:vm";
import { sample } from "./probe.mjs";
test("actual sample native getStats deadline stays distinct from outer evaluate and does not substitute empty frames", async () => {
  const context = vm.createContext({
    window: {
      __e2e: {
        peers: [
          {
            connectionState: "connected",
            getStats: () => new Promise(() => {}),
          },
        ],
      },
    },
    setTimeout,
    clearTimeout,
  });
  const collect = vm.runInContext(`(${sample.toString()})`, context);
  await assert.rejects(
    collect({ deadlineEpochMs: Date.now() + 5 }),
    (e) => e.message === "E2E_NATIVE_STATS_DEADLINE",
  );
  const actor = {
    page: {
      evaluate: async () => {
        throw new Error("E2E_NATIVE_STATS_DEADLINE");
      },
      close: async () => {},
    },
  };
  await assert.rejects(
    nativeEvaluate(actor, function sample() {}),
    (e) =>
      e.metrics.stage === "native-getStats" &&
      e.metrics.nativeDataAvailable === false,
  );
});
test("already expired absolute budget never starts probe or evaluation work", async () => {
  let called = 0;
  await assert.rejects(
    deadlineProbe(() => {
      called++;
    }, Date.now() - 1),
    NativeInterfaceFailure,
  );
  const actor = {
    page: {
      evaluate: () => {
        called++;
      },
      close: async () => {},
    },
  };
  await assert.rejects(
    nativeEvaluate(actor, () => {}, undefined, -1),
    NativeInterfaceFailure,
  );
  assert.equal(called, 0);
});
import { pollPause } from "./harness.mjs";
test("actual polling source waits out an early-waking final timer without launching a tail probe or accepting after deadline", async () => {
  let now = 0,
    calls = 0,
    accepts = 0;
  const context = vm.createContext({
    Date: { now: () => now },
    pause: async (ms) => {
      now += Math.max(1, ms - 1);
    },
    deadlineProbe: async (probe, end) => {
      assert.ok(now < end);
      return probe();
    },
    CheckFailure,
  });
  vm.runInContext(
    `const pollPause=${pollPause.toString()};const until=${until.toString()};const observe=${observe.toString()};`,
    context,
  );
  const observation = await vm.runInContext("observe", context)(
    50,
    async () => {
      calls++;
      return { frames: 7 };
    },
  );
  assert.equal(calls, 1);
  assert.equal(observation.frames, 7);
  assert.equal(now, 50);
  now = 0;
  calls = 0;
  await assert.rejects(
    vm.runInContext("until", context)(
      async () => {
        calls++;
        return { frames: 0 };
      },
      () => {
        accepts++;
        return false;
      },
      "strict-progress",
      50,
    ),
    CheckFailure,
  );
  assert.equal(calls, 1);
  assert.equal(accepts, 1);
  assert.equal(now, 50);
  now = 0;
  const pollTimes = [];
  const cadenceProbe = async () => {
    pollTimes.push(now);
    return { frames: now };
  };
  const final = await vm.runInContext("observe", context)(250, cadenceProbe);
  assert.deepEqual(pollTimes, [0, 200]);
  assert.equal(final.frames, 200);
  assert.equal(now, 250);
  now = 0;
  pollTimes.length = 0;
  await assert.rejects(
    vm.runInContext("until", context)(
      cadenceProbe,
      () => false,
      "strict-progress",
      250,
    ),
    (e) => e instanceof CheckFailure && e.metrics.last.frames === 200,
  );
  assert.deepEqual(pollTimes, [0, 200]);
  assert.equal(now, 250);
  now = 0;
  accepts = 0;
  await assert.rejects(
    vm.runInContext("until", context)(
      async () => {
        now = 51;
        return { frames: 99 };
      },
      () => {
        accepts++;
        return true;
      },
      "strict-progress",
      50,
    ),
    CheckFailure,
  );
  assert.equal(accepts, 0);
});
