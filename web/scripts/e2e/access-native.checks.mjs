/* global setTimeout, clearTimeout */
import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { heldMedia, releaseHeld, closeAccessActors } from "./access.mjs";
import { deadlineProbe, NativeInterfaceFailure } from "./native-evaluate.mjs";
function actorFor(stats, outerPending = false) {
  let evaluates = 0,
    closes = 0;
  const state = {
    heldPeers: [{ getStats: stats }],
    heldSockets: [{ ws: { readyState: 1 } }],
  };
  const context = vm.createContext({
    window: { __e2e: state },
    setTimeout,
    clearTimeout,
  });
  const actor = {
    page: {
      evaluate: async (fn, arg) => {
        evaluates++;
        return outerPending
          ? new Promise(() => {})
          : vm.runInContext(`(${fn.toString()})`, context)(arg);
      },
      close: async () => {
        closes++;
      },
      isClosed: () => false,
    },
  };
  return { actor, state, evaluates: () => evaluates, closes: () => closes };
}
const report = (frames) =>
  new Map([
    ["v", { type: "inbound-rtp", kind: "video", framesDecoded: frames }],
  ]);
test("actual heldMedia hung getStats is phase-labelled, quarantined, never fabricated as frames0 and release never reevaluates it", async () => {
  const r = actorFor(() => new Promise(() => {}));
  await assert.rejects(
    deadlineProbe(() => heldMedia(r.actor), Date.now() + 30),
    (e) =>
      e instanceof NativeInterfaceFailure &&
      e.metrics.stage === "native-getStats" &&
      e.metrics.nativeDataAvailable === false,
  );
  const count = r.evaluates();
  await releaseHeld(r.actor);
  await r.actor.nativeAbortClose;
  assert.equal(r.evaluates(), count);
  assert.equal(r.closes(), 1);
});
test("actual heldMedia pending outer evaluate cannot bypass quarantine or trigger a second probe", async () => {
  const r = actorFor(() => Promise.resolve(report(5)), true);
  await assert.rejects(
    deadlineProbe(() => heldMedia(r.actor), Date.now() + 5),
    NativeInterfaceFailure,
  );
  await assert.rejects(
    heldMedia(r.actor),
    (e) => e.metrics.stage === "page-quarantined",
  );
  await releaseHeld(r.actor);
  assert.equal(r.evaluates(), 1);
  assert.equal(r.closes(), 1);
});
test("actual heldMedia rejects late native answer but records normal increasing and post-revocation stagnant frames unchanged", async () => {
  const late = actorFor(
    () => new Promise((resolve) => setTimeout(() => resolve(report(99)), 30)),
  );
  await assert.rejects(
    deadlineProbe(() => heldMedia(late.actor), Date.now() + 5),
    NativeInterfaceFailure,
  );
  await new Promise((resolve) => setTimeout(resolve, 35));
  await assert.rejects(heldMedia(late.actor), NativeInterfaceFailure);
  let frames = 10;
  const normal = actorFor(() => Promise.resolve(report(frames)));
  const a = await heldMedia(normal.actor);
  frames = 20;
  const b = await heldMedia(normal.actor);
  normal.state.heldSockets[0].ws.readyState = 3;
  const after = await heldMedia(normal.actor);
  assert.equal(a.frames, 10);
  assert.equal(b.frames, 20);
  assert.equal(after.frames, b.frames);
  assert.equal(after.openSockets, 0);
});
test("actual access finally attempts every own context after restore rejection and close timeout, with redacted failure stages", async () => {
  const actions = [];
  const bad = {
    page: {
      evaluate: async () => {
        actions.push("restore");
        throw new Error("PRIVATE-restore");
      },
      isClosed: () => false,
    },
    context: {
      close: () => {
        actions.push("close0");
        return new Promise(() => {});
      },
    },
  };
  const healthy = {
    context: {
      close: async () => {
        actions.push("close1");
      },
    },
  };
  await assert.rejects(
    closeAccessActors([bad, healthy], [bad], 5),
    (e) =>
      e instanceof NativeInterfaceFailure &&
      e.metrics.stage === "access-restore-or-context-close" &&
      e.metrics.failedSteps.join(",") === "held-restore-0,context-close-0" &&
      !JSON.stringify(e.metrics).includes("PRIVATE"),
  );
  assert.deepEqual(actions, ["restore", "close0", "close1"]);
});
import { closeLeaseFixture } from "./media-lease-faults.mjs";
test("actual lease finally restores Redis and closes every own context even when API resume and a close fail", async () => {
  const actions = [];
  const actor = (i) => ({
    nativeEvaluationUnusable: true,
    context: {
      close: async () => {
        actions.push("close" + i);
        if (i === 0) throw new Error("PRIVATE-close");
      },
    },
  });
  const f = { owner: actor(0), member: actor(1), watcher: actor(2) };
  const h = {
    faultRuntime: {
      resumeApi: () => {
        actions.push("api");
        throw new Error("PRIVATE-api");
      },
      redis: {
        restore: () => {
          actions.push("redis");
        },
      },
    },
  };
  await assert.rejects(
    closeLeaseFixture(h, f, false),
    (e) =>
      e instanceof NativeInterfaceFailure &&
      e.metrics.stage === "lease-owned-restore" &&
      !JSON.stringify(e.metrics).includes("PRIVATE"),
  );
  assert.deepEqual(actions, ["api", "redis", "close0", "close1", "close2"]);
});
