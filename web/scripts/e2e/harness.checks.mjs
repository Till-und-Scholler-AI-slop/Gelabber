/* global URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CheckFailure,
  check,
  failureSite,
  revision,
  safeError,
  safeTarget,
  until,
} from "./harness.mjs";
import { decoded, interrupt } from "./media.mjs";
import { instrument } from "./probe.mjs";
import { createContext, runInContext } from "node:vm";
import {
  failureEvidence,
  harnessSite,
  stepLog,
  timedOutAction,
} from "./harness.mjs";

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
test("a failure names the scenario's line, not the harness helper", () => {
  const stack = [
    "CheckFailure: x",
    "    at until (file:///w/web/scripts/e2e/harness.mjs:45:9)",
    "    at async f (file:///w/web/scripts/e2e/media-lease-faults.mjs:66:24)",
    "    at async run (file:///w/web/scripts/e2e/harness.mjs:354:18)",
  ].join("\n");
  assert.equal(failureSite(stack), "media-lease-faults.mjs:66:24");
  assert.equal(
    failureSite("    at x (/w/web/scripts/e2e/harness.mjs:286:10)"),
    "harness.mjs:286:10",
  );
  assert.equal(failureSite(undefined), null);
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
test("a browser deadline names the Playwright call that ran out, nothing else", () => {
  const e = new Error(
    'page.goto: Timeout 12000ms exceeded.\nCall log:\n  - navigating to "http://127.0.0.1:15186/register?token=SECRET"',
  );
  e.name = "TimeoutError";
  assert.equal(timedOutAction(e), "page.goto");
  e.message = "locator.click: Timeout 12000ms exceeded.\nCall log: SECRET";
  assert.equal(timedOutAction(e), "locator.click");
  e.message = "https://host/?token=SECRET: Timeout 1ms exceeded";
  assert.equal(timedOutAction(e), null);
  assert.equal(
    timedOutAction(new Error("page.goto: Timeout 12000ms exceeded.")),
    null,
  );
  // The harness line too: one scenario line can call several harness steps.
  const stack = [
    "TimeoutError: x",
    "    at channel (file:///w/web/scripts/e2e/harness.mjs:297:7)",
    "    at async fresh (file:///w/web/scripts/e2e/chat-faults.mjs:43:16)",
  ].join("\n");
  assert.equal(harnessSite(stack), "harness.mjs:297:7");
  assert.equal(harnessSite(undefined), null);
});
test("a failed set-up row lists each step's time and the call that ran out", async (t) => {
  t.mock.timers.enable({ apis: ["Date"], now: 0 });
  const { steps, step } = stepLog();
  assert.equal(
    await step("Publisher-register-page", async () => {
      t.mock.timers.setTime(1_500);
      return "page";
    }),
    "page",
  );
  const deadline = new Error(
    'page.waitForURL: Timeout 12000ms exceeded.\nCall log:\n  - waiting for "http://127.0.0.1:15186/s/1?invite=SECRET"',
  );
  deadline.name = "TimeoutError";
  deadline.stack = [
    "TimeoutError: page.waitForURL: Timeout 12000ms exceeded.",
    "    at step (file:///w/web/scripts/e2e/harness.mjs:150:22)",
    "    at async fixture (file:///w/web/scripts/e2e/harness.mjs:286:7)",
    "    at async file:///w/web/scripts/e2e/run.mjs:42:9",
  ].join("\n");
  await assert.rejects(
    step("server-create", async () => {
      t.mock.timers.setTime(13_600);
      throw deadline;
    }),
    (error) => error === deadline,
  );
  assert.deepEqual(steps, [
    { step: "Publisher-register-page", ms: 1_500 },
    { step: "server-create", ms: 12_100 },
  ]);
  deadline.steps = steps;
  const evidence = failureEvidence(deadline);
  assert.deepEqual(evidence, {
    site: "run.mjs:42:9",
    action: "page.waitForURL",
    harnessSite: "harness.mjs:150:22",
    steps,
  });
  assert.ok(!JSON.stringify(evidence).includes("SECRET"));
  // A check's own failure names no Playwright call.
  assert.deepEqual(
    Object.keys(failureEvidence(new CheckFailure("fixture-x"))),
    ["site"],
  );
});
