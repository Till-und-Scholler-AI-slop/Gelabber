/* global URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CheckFailure,
  check,
  revision,
  safeError,
  safeTarget,
  until,
} from "./harness.mjs";
import { decoded, interrupt } from "./media.mjs";
import { instrument } from "./probe.mjs";
import { createContext, runInContext } from "node:vm";

test("bytes and closed receivers cannot pass decoded-frame acceptance", () => {
  assert.equal(
    decoded({
      peers: [
        {
          connection: "connected",
          inbound: [{ kind: "video", bytes: 9000, frames: 0 }],
        },
      ],
    }),
    0,
  );
  assert.equal(
    decoded({
      peers: [
        { connection: "closed", inbound: [{ kind: "video", frames: 300 }] },
      ],
    }),
    0,
  );
  assert.equal(
    decoded({
      peers: [
        {
          connection: "connected",
          inbound: [
            { kind: "audio", frames: 80 },
            { kind: "video", frames: 4 },
          ],
        },
      ],
    }),
    4,
  );
});
test("evidence errors discard Playwright URLs, secrets and call logs", () => {
  const e = new Error(
    "cookie=SECRET signed https://host/?token=SECRET email/private message/SDP",
  );
  assert.equal(safeError(e), "harness-or-interface-error");
  e.name = "TimeoutError";
  assert.equal(safeError(e), "browser-deadline");
  assert.equal(
    safeError(new CheckFailure("decoded-frame-deadline")),
    "decoded-frame-deadline",
  );
  assert.equal(revision("secret"), "unknown");
  assert.equal(revision("828ee23"), "828ee23");
});
test("11a account-creating runner refuses production and credential-bearing URLs", () => {
  for (const url of [
    "https://sscholler.de",
    "http://user:secret@localhost",
    "http://localhost?token=secret",
    "file:///tmp/app",
  ])
    assert.throws(() => safeTarget(url));
  assert.equal(safeTarget("http://127.0.0.1:5174"), "http://127.0.0.1:5174");
});
test("deadline failure preserves safe measured counters instead of turning red into skip", async (t) => {
  // Expire the clock after a real sample, not before a scheduler-dependent 1ms probe.
  t.mock.timers.enable({ apis: ["Date"], now: 0 });
  await assert.rejects(
    until(
      async () => ({ frames: 0, bytes: 200 }),
      (s) => {
        t.mock.timers.setTime(1);
        return s.frames > 0;
      },
      "no-decoded-progress",
      1,
    ),
    (e) =>
      e instanceof CheckFailure &&
      e.message === "no-decoded-progress" &&
      e.metrics.last.frames === 0 &&
      e.metrics.last.bytes === 200,
  );
  assert.throws(() => check(false, "missing-predecessor"), CheckFailure);
});

test("gateway faults leave HMR, media and unrelated socket paths untouched", async () => {
  class Socket {
    constructor(url) {
      this.url = String(url);
      this.readyState = 1;
      this.listeners = {};
    }
    addEventListener(event, fn) {
      this.listeners[event] = fn;
    }
    send() {}
    close() {
      this.readyState = 3;
    }
  }
  const window = {
    RTCPeerConnection: class {},
    WebSocket: Socket,
    location: { href: "http://127.0.0.1:5174/" },
    HTMLMediaElement: class {
      play() {
        return Promise.resolve();
      }
    },
  };
  const context = createContext({
    window,
    URL,
    navigator: { mediaDevices: { getUserMedia: async () => {} } },
  });
  runInContext(`(${instrument.toString()})({ relay: false })`, context);
  for (const path of [
    "/ws",
    "/media/ws",
    "/?token=synthetic-hmr",
    "/media/debug",
    "/ws/",
    "/socket",
  ])
    new window.WebSocket(`ws://127.0.0.1:5174${path}`);
  assert.deepEqual(
    [...window.__e2e.sockets].map((s) => s.plane),
    ["gateway", "media", "other", "other", "other", "other"],
  );
  const gateway = window.__e2e.sockets[0];
  for (const frame of [
    { op: "ok", c: "synthetic-topic", n: 12, ep: "old" },
    { op: "e", c: "synthetic-topic", n: 1, ep: "new" },
    { op: "gap", c: "synthetic-topic", n: 1, ep: "new" },
    { op: "resync" },
  ])
    gateway.ws.listeners.message({ data: JSON.stringify(frame) });
  assert.equal(gateway.topics.get("synthetic-topic").epochChanges, 1);
  assert.equal(gateway.receivedEvents, 1);
  assert.equal(gateway.gaps, 1);
  assert.equal(gateway.resyncs, 1);
  const actor = {
    page: {
      evaluate: (fn, value) => {
        context.argument = value;
        return runInContext(`(${fn.toString()})(argument)`, context);
      },
    },
  };
  assert.equal(await interrupt(actor, "gateway"), 1);
  assert.deepEqual(
    [...window.__e2e.sockets].map((s) => s.ws.readyState),
    [3, 1, 1, 1, 1, 1],
  );
  assert.equal(await interrupt(actor, "media"), 1);
  assert.deepEqual(
    [...window.__e2e.sockets].map((s) => s.ws.readyState),
    [3, 3, 1, 1, 1, 1],
  );
  await assert.rejects(interrupt(actor, "other"), /unsupported-fault-plane/);
});

const { selection, wanted, requiredCase, gate } =
  await import("./selection.mjs");
test("explicit exclusions are NOT_RUN, required failures and incomplete selection stay red", () => {
  const selected = selection("good-case");
  assert.equal(wanted(selected, "good-case"), true);
  assert.equal(requiredCase(selected, "fixture", true), true);
  assert.equal(requiredCase(selected, "other-case"), false);
  const rows = [
    { id: "fixture", status: "PASS" },
    { id: "good-case", status: "PASS" },
    { id: "other-case", status: "NOT_RUN" },
  ];
  assert.equal(gate(rows, selected, [{ status: 204 }]).passed, true);
  assert.equal(gate(rows, selected, []).completeAcceptance, false);
  for (const status of ["FAIL", "BLOCKED"])
    assert.equal(
      gate([...rows, { id: "required", status }], selected, []).passed,
      false,
    );
  assert.equal(gate(rows, selected, [{ status: "FAILED" }]).passed, false);
  assert.equal(
    gate(
      [...rows, { id: "restore", status: "PASS", fixtureRecovery: "BLOCKED" }],
      selected,
      [],
    ).passed,
    false,
  );
  assert.deepEqual(gate(rows, ["typo-case"], []).absent, ["typo-case"]);
  assert.equal(gate(rows, ["typo-case"], []).passed, false);
  assert.equal(gate([], [], []).passed, false);
  assert.throws(() => selection("good-case,good-case"));
  assert.throws(() => selection("bad/secret"));
});
