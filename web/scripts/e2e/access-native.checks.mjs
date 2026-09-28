/* global setTimeout, clearTimeout, URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import {
  heldMedia,
  holdActiveMedia,
  releaseHeld,
  closeAccessActors,
} from "./access.mjs";
import { deadlineProbe, NativeInterfaceFailure } from "./native-evaluate.mjs";
import { CheckFailure } from "./harness.mjs";
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
for (const [label, stats] of [
  ["empty stats", new Map()],
  ["no video RTP", new Map([["a", { type: "inbound-rtp", kind: "audio" }]])],
  ["missing counter", report(undefined)],
  ["invalid counter", report(NaN)],
]) {
  test(`actual heldMedia rejects ${label} instead of inventing a decoded stop`, async () => {
    const r = actorFor(async () => stats);
    await assert.rejects(
      heldMedia(r.actor),
      (error) =>
        error instanceof CheckFailure &&
        error.message === "fixture-held-video-counters-unavailable" &&
        error.metrics.frames === null &&
        error.metrics.frameCountersAvailable === false,
    );
  });
}
test("actual heldMedia rejects partial counter disappearance and replacement after a valid control", async () => {
  let stats = new Map([
    ...report(20),
    [
      "second-video",
      {
        type: "inbound-rtp",
        kind: "video",
        framesDecoded: 10,
      },
    ],
  ]);
  const r = actorFor(async () => stats);
  const before = await heldMedia(r.actor);
  assert.equal(before.frames, 30);
  stats = report(20);
  await assert.rejects(heldMedia(r.actor), (e) => e.metrics?.frames === null);
  stats = new Map([
    ...report(20),
    [
      "replacement-video",
      {
        type: "inbound-rtp",
        kind: "video",
        framesDecoded: 10,
      },
    ],
  ]);
  await assert.rejects(heldMedia(r.actor), (e) => e.metrics?.frames === null);
});
test("actual heldMedia preserves available numeric zero and known stagnant counters", async () => {
  const zero = await heldMedia(actorFor(async () => report(0)).actor);
  assert.equal(zero.frames, 0);
  const known = actorFor(async () => report(20));
  assert.equal((await heldMedia(known.actor)).frames, 20);
  assert.equal((await heldMedia(known.actor)).frames, 20);
});
test("actual hold establishes known positive video counters before any fault and rejects unavailable/zero fixtures", async () => {
  const make = (stats) => {
    const r = actorFor(async () => stats);
    r.state.peers = r.state.heldPeers;
    r.state.peers[0].connectionState = "connected";
    r.state.sockets = [];
    return r.actor;
  };
  const healthy = make(report(20));
  await holdActiveMedia(healthy);
  assert.equal((await heldMedia(healthy)).frames, 20);
  for (const stats of [new Map(), report(0)])
    await assert.rejects(holdActiveMedia(make(stats)), CheckFailure);
});
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

import { readFile } from "node:fs/promises";
import { check } from "./harness.mjs";
const accessSource = await readFile(
  new URL("./access.mjs", import.meta.url),
  "utf8",
);
const leaseSource = await readFile(
  new URL("./media-lease-faults.mjs", import.meta.url),
  "utf8",
);
const stopChecks = {
  revocation: accessSource.match(
    /check\(\s*after\.frames === settled\.frames[\s\S]*?"revoked-client-retained-access",\s*metrics,\s*\);/,
  )[0],
  deletion: accessSource
    .slice(
      accessSource.indexOf(
        "const held = await useHeldTicket(victim);",
        accessSource.indexOf("channel-server-delete-active-sockets"),
      ),
    )
    .match(/check\([\s\S]*?\n\s*\);/)[0],
  lease: leaseSource.match(
    /check\(\s*tail\.frames === stopped\.frames,[\s\S]*?\n\s*\);/,
  )[0],
};
for (const [boundary, source] of Object.entries(stopChecks)) {
  for (const kind of ["known", "empty", "missing-counter"]) {
    test(`actual ${boundary} stop check ${kind === "known" ? "accepts known stagnant counters" : "rejects " + kind}`, async () => {
      const r = actorFor(async () =>
        kind === "empty"
          ? new Map()
          : report(kind === "known" ? 20 : undefined),
      );
      r.state.heldSockets[0].ws.readyState = 3;
      let accepted = false;
      try {
        const settled = await heldMedia(r.actor),
          after = await heldMedia(r.actor);
        vm.runInNewContext(source, {
          check,
          settled,
          after,
          stopped: settled,
          tail: after,
          metrics: {},
          gateway: { events: 0 },
          beforeGateway: { events: 0 },
          rest: { status: 403 },
          freshTicket: { status: 403 },
          fresh: { status: 403 },
          held: { failed: false, accepted: false },
          scope: "channel",
          plane: "redis",
        });
        accepted = true;
      } catch (error) {
        assert.ok(error instanceof CheckFailure);
      }
      assert.equal(accepted, kind === "known");
    });
  }
}

import * as nativeInterface from "./native-evaluate.mjs";
import * as teardown from "./teardown.mjs";
async function independentSessionControl({
  ownerLogoutStatus = 200,
  ownerSessionStatus = 200,
  ownerStillSignedIn = false,
  beforeStatus = 200,
  afterStatus = 200,
  cleanupLogoutStatus = 200,
} = {}) {
  const actions = [],
    owner = {
      id: "owned-user",
      password: "synthetic-only",
      email: "synthetic@example.invalid",
    };
  let result,
    error,
    reads = 0;
  const locator = { fill: async () => {}, click: async () => {} };
  const page = {
    goto: async () => {},
    waitForURL: async () => {},
    getByLabel: () => locator,
    getByRole: () => locator,
  };
  const mocks = {
    "./harness.mjs": {
      check,
      click: async () => {},
      navigate: async () => {},
      observe: async () => {},
      until: async () => {},
      api: async (actor, path, method) => {
        if (path === "/auth/session") {
          if (actor === owner) {
            actions.push("owner.session.after");
            return {
              status: ownerSessionStatus,
              body: { user: ownerStillSignedIn ? { id: owner.id } : null },
            };
          }
          actions.push(
            reads ? "independent.session.after" : "independent.session.before",
          );
          return {
            status: reads++ ? afterStatus : beforeStatus,
            body: { user: { id: owner.id } },
          };
        }
        assert.equal(path, "/auth/logout");
        assert.equal(method, "POST");
        actions.push(actor === owner ? "owner.logout" : "independent.logout");
        return {
          status: actor === owner ? ownerLogoutStatus : cleanupLogoutStatus,
        };
      },
    },
    "./native-evaluate.mjs": nativeInterface,
    "./teardown.mjs": teardown,
    "./media.mjs": { progress: async () => {} },
  };
  const context = vm.createContext({});
  const source = new vm.SourceTextModule(
    await readFile(new URL("./access.mjs", import.meta.url), "utf8"),
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
    setIsolation: () => {},
    browser: {
      newContext: async () => ({
        newPage: async () => page,
        close: async () => {
          actions.push("independent.context.close");
        },
      }),
    },
    run: async (id, _predecessors, task) => {
      if (id === "logout-other-independent-session-survives") {
        try {
          result = await task();
        } catch (e) {
          error = e;
        }
      }
    },
  };
  await source.namespace.accessScenarios(h, {
    owner,
    textPath: "/s/control/c/chat",
    voicePath: "/s/control/c/voice",
    base: "http://127.0.0.1:15186",
  });
  return { result, error, actions };
}
test("actual independent-session task requires successful owner logout and retains/cleans the other session", async () => {
  const r = await independentSessionControl();
  assert.equal(r.error, undefined);
  assert.equal(r.result.independentSessionRetained, true);
  assert.ok(r.actions.includes("independent.logout"));
  assert.ok(r.actions.includes("independent.context.close"));
});
for (const options of [
  { ownerLogoutStatus: 503 },
  { ownerLogoutStatus: 401 },
  { ownerStillSignedIn: true },
  { ownerSessionStatus: 503 },
  { beforeStatus: 503 },
  { afterStatus: 503 },
  { cleanupLogoutStatus: 503 },
]) {
  test(`actual independent-session task rejects an unexercised/failed boundary ${JSON.stringify(options)}`, async () => {
    const r = await independentSessionControl(options);
    assert.ok(r.error);
    assert.equal(r.result?.independentSessionRetained, undefined);
    assert.ok(r.actions.includes("independent.logout"));
    assert.ok(r.actions.includes("independent.context.close"));
    if (options.ownerLogoutStatus)
      assert.ok(!r.actions.includes("independent.session.after"));
  });
}
