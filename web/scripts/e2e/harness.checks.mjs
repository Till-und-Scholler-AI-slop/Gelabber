/* global URL, process, setImmediate */
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CheckFailure,
  check,
  failureSite,
  responseTo,
  revision,
  safeError,
  safeTarget,
  until,
} from "./harness.mjs";
import { decoded, interrupt } from "./media.mjs";
import { chatFaultScenarios } from "./chat-faults.mjs";
import { profiles } from "./profiles.mjs";
import { instrument } from "./probe.mjs";
import { createContext, runInContext } from "node:vm";
import {
  failureEvidence,
  harnessSite,
  stepLog,
  timedOutAction,
  inviteRequests,
  invitePageState,
  chatRequests,
  chatPageState,
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
test("a response waiter whose action failed never rejects unheard", async () => {
  const unheard = [];
  const listen = (reason) => unheard.push(reason);
  process.on("unhandledRejection", listen);
  try {
    let expire;
    const page = {
      waitForResponse: () =>
        new Promise((_, reject) => {
          expire = reject;
        }),
    };
    await assert.rejects(
      responseTo(
        page,
        () => true,
        async () => {
          throw new Error("click-failed");
        },
      ),
      /click-failed/,
    );
    // The waiter's own deadline passes after the scenario already failed.
    expire(new Error("waitForResponse timed out"));
    await new Promise((done) => setImmediate(done));
  } finally {
    process.off("unhandledRejection", listen);
  }
  assert.deepEqual(unheard, []);
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
  // The API's class name can be camelCase.
  e.message = "browserType.connect: Timeout 1ms exceeded.";
  assert.equal(timedOutAction(e), "browserType.connect");
  e.message = "browserContext.newPage: Timeout 12000ms exceeded.";
  assert.equal(timedOutAction(e), "browserContext.newPage");
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
  // A waiter's timeout starts in responseTo (GEL-453); its caller is the step.
  const waiter = [
    "TimeoutError: page.waitForResponse: Timeout 12000ms exceeded",
    "    at responseTo (file:///w/web/scripts/e2e/harness.mjs:47:46)",
    "    at async channel (file:///w/web/scripts/e2e/harness.mjs:331:24)",
    "    at async fresh (file:///w/web/scripts/e2e/chat-faults.mjs:43:16)",
  ].join("\n");
  assert.equal(harnessSite(waiter), "harness.mjs:331:24");
  assert.equal(
    harnessSite(
      "    at async step (file:///w/web/scripts/e2e/harness.mjs:105:14)",
    ),
    null,
  );
  assert.equal(harnessSite(undefined), null);
});
test("a failed step is listed with its time; its row names the call that ran out", async (t) => {
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
    "    at file:///w/web/scripts/e2e/harness.mjs:262:18",
    "    at async step (file:///w/web/scripts/e2e/harness.mjs:105:14)",
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
  const evidence = failureEvidence(deadline);
  assert.deepEqual(evidence, {
    site: "run.mjs:42:9",
    action: "page.waitForURL",
    harnessSite: "harness.mjs:262:18",
  });
  assert.ok(!JSON.stringify(evidence).includes("SECRET"));
  // A check's own failure names no Playwright call.
  assert.deepEqual(
    Object.keys(failureEvidence(new CheckFailure("fixture-x"))),
    ["site"],
  );
});
test("a listed expected failure keeps the gate green only while it fails for its reason", () => {
  const expected = {
    known: { issue: "GEL-1", reason: "known-reason" },
    pinned: {
      issue: "GEL-2",
      reason: "browser-deadline",
      site: "media.mjs:431",
    },
  };
  const rows = [{ id: "fixture", status: "PASS" }];
  const run = (row) =>
    gate([...rows, row], [row.id], [{ status: 204 }], expected);
  const failing = run({ id: "known", status: "FAIL", reason: "known-reason" });
  assert.equal(failing.passed, true);
  assert.deepEqual(failing.expectedFailures, [
    { id: "known", issue: "GEL-1", reason: "known-reason" },
  ]);
  assert.equal(
    run({
      id: "pinned",
      status: "FAIL",
      reason: "browser-deadline",
      site: "media.mjs:431:15",
    }).passed,
    true,
  );
  const passing = run({ id: "known", status: "PASS" });
  assert.equal(passing.passed, false);
  assert.deepEqual(passing.nowPassing, [{ id: "known", issue: "GEL-1" }]);
  const otherReason = run({
    id: "known",
    status: "FAIL",
    reason: "another-reason",
    site: "media.mjs:9:1",
  });
  assert.equal(otherReason.passed, false);
  assert.deepEqual(otherReason.otherFailures, [
    {
      id: "known",
      issue: "GEL-1",
      reason: "another-reason",
      site: "media.mjs:9:1",
    },
  ]);
  for (const row of [
    // The pinned reason at another line is another failure.
    {
      id: "pinned",
      status: "FAIL",
      reason: "browser-deadline",
      site: "media.mjs:4310:2",
    },
    { id: "known", status: "BLOCKED" },
    { id: "other", status: "FAIL", reason: "known-reason" },
  ])
    assert.equal(run(row).passed, false);
});
test("every expected failure of a profile names one of its cases, its issue and its reason", () => {
  for (const [name, profile] of Object.entries(profiles))
    for (const [id, entry] of Object.entries(profile.expectedFailures ?? {})) {
      assert.ok(
        profile.cases.includes(id),
        `${name}: ${id} is not one of its cases`,
      );
      assert.match(entry.issue, /^(GEL-\d+|#\d+)$/, `${name}: ${id}`);
      assert.match(entry.reason, /^[a-z0-9-]+$/, `${name}: ${id}`);
    }
});
test("a failed invite step can say which invite requests went out and how they ended", () => {
  const listeners = {};
  const page = { on: (event, listener) => (listeners[event] = listener) };
  let clock = 1_000;
  const requests = inviteRequests(page, () => clock);
  const request = (method, path) => ({
    method: () => method,
    url: () => `http://127.0.0.1:15186${path}`,
  });
  const create = request("POST", "/api/servers/s-1/invites");
  const revoke = request("DELETE", "/api/invites/CODE");
  const other = request("GET", "/api/servers/s-1");
  clock = 1_200;
  listeners.request(create);
  listeners.request(other);
  clock = 1_350;
  listeners.request(revoke);
  clock = 1_400;
  listeners.response({ request: () => create, status: () => 201 });
  clock = 1_500;
  listeners.requestfailed(revoke);
  assert.deepEqual(requests.list(), [
    { method: "POST", sentAt: 200, status: 201, failed: false, doneAt: 400 },
    { method: "DELETE", sentAt: 350, status: null, failed: true, doneAt: 500 },
  ]);
});
test("a failed invite step can say what the page showed: dialogs and the link field", () => {
  // A role="dialog" element, or the app's native <dialog> (Modal) when
  // `open` is given.
  const dialog = (label, text, open) => ({
    tagName: open === undefined ? "DIV" : "DIALOG",
    open,
    getAttribute: (name) => (name === "aria-label" ? label : null),
    querySelector: () => null,
    textContent: text,
  });
  const state = (field, dialogs) =>
    runInContext(
      `(${invitePageState.toString()})()`,
      createContext({
        location: {
          pathname: "/s/0b6b8e1e-6f0a-4c8e-9d3a-2a6c0f1e9b7d/c/x",
        },
        document: {
          querySelector: () => field,
          querySelectorAll: () => dialogs,
        },
      }),
    );
  const failed = state(null, [
    dialog(
      "E2E einladen",
      "  Einladungslink konnte nicht erstellt werden.\n Erneut versuchen ",
    ),
    dialog("Server erstellen", "Name", false),
  ]);
  assert.equal(failed.path, "/s/:id/c/x");
  assert.equal(failed.field, "missing");
  assert.deepEqual(JSON.parse(JSON.stringify(failed.dialogs)), [
    {
      open: null,
      name: "E2E einladen",
      text: "Einladungslink konnte nicht erstellt werden. Erneut versuchen",
    },
    { open: false, name: "Server erstellen", text: "Name" },
  ]);
  assert.equal(state({ value: "" }, []).field, "empty");
  assert.equal(state({ value: "http://h/invite/ABC" }, []).field, "link");
});

test("a chat fault's request log keeps the server's and the chat's requests, IDs masked, until it stops", async () => {
  const listeners = new Map();
  const page = {
    on: (event, listener) => listeners.set(event, listener),
    off: (event, listener) => {
      if (listeners.get(event) === listener) listeners.delete(event);
    },
  };
  let clock = 0;
  const requests = chatRequests(page, () => clock);
  const id = "0b6b8e1e-6f0a-4c8e-9d3a-2a6c0f1e9b7d";
  const request = (method, path) => ({
    method: () => method,
    url: () => `http://127.0.0.1:15186/api${path}?limit=50`,
  });
  const detail = request("GET", `/servers/${id}`);
  const create = request("POST", `/servers/${id}/channels`);
  const send = request("POST", `/channels/${id}/messages`);
  const upload = request("PUT", `/attachments/${id}/upload`);
  clock = 100;
  for (const r of [detail, create, send, upload]) listeners.get("request")(r);
  // Pictures and the session are not the chat's.
  listeners.get("request")(request("GET", `/attachments/${id}`));
  listeners.get("request")(request("GET", "/auth/session"));
  const answer = (r, status, body) =>
    listeners.get("response")({
      request: () => r,
      status: () => status,
      json: async () => body,
    });
  clock = 250;
  answer(create, 201, { id: "created-channel" });
  clock = 400;
  listeners.get("requestfailed")(send);
  // A read without the created channel, and one with it.
  const before = request("GET", `/servers/${id}`);
  listeners.get("request")(before);
  answer(detail, 200, { channels: [{ id: "a" }, { id: "b" }] });
  answer(before, 200, { channels: [{ id: "a" }, { id: "created-channel" }] });
  await new Promise(setImmediate);
  assert.deepEqual(
    requests
      .list()
      .map(({ path, method, status, failed, channels, holdsCreated }) => [
        method,
        path,
        status,
        failed,
        channels,
        holdsCreated,
      ]),
    [
      ["GET", "/api/servers/:id", 200, false, 2, false],
      ["POST", "/api/servers/:id/channels", 201, false, undefined, undefined],
      ["POST", "/api/channels/:id/messages", null, true, undefined, undefined],
      ["PUT", "/api/attachments/:id/upload", null, false, undefined, undefined],
      ["GET", "/api/servers/:id", 200, false, 2, true],
    ],
  );
  // Counts and a yes or no only: no channel's ID leaves the page.
  assert.ok(!JSON.stringify(requests.list()).includes("created-channel"));
  requests.stop();
  assert.equal(listeners.size, 0);
});

test("a chat fault scenario that fails records both pages' requests and screens, and stops recording", async () => {
  const id = "0b6b8e1e-6f0a-4c8e-9d3a-2a6c0f1e9b7d";
  const page = (who) => {
    const listeners = new Map();
    return {
      listeners,
      on: (event, listener) => listeners.set(event, listener),
      off: (event, listener) => {
        if (listeners.get(event) === listener) listeners.delete(event);
      },
      // The owner's first page: the server is read, then it runs out.
      goto: async () => {
        const read = {
          method: () => "GET",
          url: () => `http://127.0.0.1:15186/api/servers/${id}`,
        };
        listeners.get("request")(read);
        listeners.get("response")({
          request: () => read,
          status: () => 200,
          json: async () => ({ channels: [{ id: "a" }] }),
        });
        const error = new Error("page.goto: Timeout 12000ms exceeded.");
        error.name = "TimeoutError";
        throw error;
      },
      evaluate: async () => ({ path: `/s/:id/${who}` }),
    };
  };
  const f = {
    owner: { page: page("owner") },
    member: { page: page("member") },
    textPath: "/s/x/c/y",
    base: "http://127.0.0.1:15186",
  };
  let failure;
  await chatFaultScenarios(
    {
      step: (_name, action) => action(),
      run: async (scenario, _predecessors, task) => {
        if (scenario !== "two-parallel-sends-navigation-retry") return;
        await task().catch((error) => {
          failure = error;
        });
      },
    },
    f,
  );
  assert.equal(failure?.name, "TimeoutError");
  const evidence = JSON.parse(JSON.stringify(failure.evidence));
  assert.deepEqual(
    evidence.chatRequests.owner.map(({ method, path, channels }) => [
      method,
      path,
      channels,
    ]),
    [["GET", "/api/servers/:id", 1]],
  );
  assert.deepEqual(evidence.chatRequests.member, []);
  assert.deepEqual(evidence.chatPages, {
    owner: { path: "/s/:id/owner" },
    member: { path: "/s/:id/member" },
  });
  for (const who of [f.owner, f.member])
    assert.equal(who.page.listeners.size, 0);
});

test("a chat fault can say what the page showed: pending channels and each message's state", () => {
  const element = (tagName, text, { classes = [], children = {} } = {}) => ({
    tagName,
    textContent: text,
    classList: { contains: (name) => classes.includes(name) },
    querySelector: (selector) => children[selector] ?? null,
    querySelectorAll: (selector) => children[selector] ?? [],
  });
  const button = (text) => element("BUTTON", text);
  const message = (text, classes, buttons = []) =>
    element("DIV", text, {
      classes,
      children: {
        ".lr-message-text": element("SPAN", text),
        button: buttons.map(button),
      },
    });
  const nodes = {
    'dialog[open],[role="dialog"],[role="alertdialog"]': [
      element("DIALOG", " Kanal  erstellen "),
    ],
    ".channel-sidebar .channel-row-link": [
      element("A", "e2e-voice-a"),
      element("SPAN", "e2e-parallel-attempts"),
    ],
    ".lr-message-row": [
      message("E2E sent", []),
      message("E2E on its way", ["is-sending"]),
      message("E2E turned down", ["is-failed"], ["Verwerfen", "Erneut senden"]),
    ],
  };
  const state = runInContext(
    `(${chatPageState.toString()})()`,
    createContext({
      location: { pathname: "/s/0b6b8e1e-6f0a-4c8e-9d3a-2a6c0f1e9b7d/c/x" },
      document: { querySelectorAll: (selector) => nodes[selector] ?? [] },
    }),
  );
  assert.deepEqual(JSON.parse(JSON.stringify(state)), {
    path: "/s/:id/c/x",
    dialogs: ["Kanal erstellen"],
    channels: [
      { name: "e2e-voice-a", link: true },
      { name: "e2e-parallel-attempts", link: false },
    ],
    messages: [
      { text: "E2E sent", sending: false, failed: false, retry: false },
      { text: "E2E on its way", sending: true, failed: false, retry: false },
      { text: "E2E turned down", sending: false, failed: true, retry: true },
    ],
  });
});
