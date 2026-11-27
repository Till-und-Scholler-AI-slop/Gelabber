/* global URL, setImmediate */
import { test } from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import * as native from "./native-evaluate.mjs";
import * as options from "./browser-options.mjs";
async function finishProbe(
  brokenClose = false,
  stopRejected = false,
  actorClose,
) {
  const actions = [],
    reports = [];
  let actorEvaluations = 0,
    index = 0;
  const locator = {
    fill: async () => {},
    click: async () => {},
    first() {
      return this;
    },
    getByRole() {
      return this;
    },
    getByLabel() {
      return this;
    },
    getByText() {
      return this;
    },
    inputValue: async () => "http://127.0.0.1:5174/invite/control",
  };
  const browser = {
    version: () => "155.0",
    newContext: async () => {
      const actorIndex = index++;
      let pageClosed = false;
      actions.push("context-created");
      const page = {
        goto: async () => {},
        waitForURL: async () => {},
        waitForFunction: async () => {},
        url: () => "http://127.0.0.1:5174/s/server-control/c/text-control",
        setDefaultTimeout() {},
        isClosed: () => pageClosed,
        getByRole: () => locator,
        getByLabel: () => locator,
        locator: () => locator,
        waitForResponse: async () => ({
          status: () => 200,
          json: async () => ({ id: "voice-control" }),
        }),
        evaluate: async (fn, arg) => {
          if (actorIndex === 0 && pageClosed) {
            actions.push("closed-actor-evaluate-attempt");
            throw new Error("PRIVATE-closed-page-error");
          }
          if (stopRejected && actorIndex === 0 && arg === undefined) {
            actions.push("healthy-stop-evaluate-rejected");
            throw new Error("PRIVATE-native-close-error");
          }
          if (actorIndex === 0 && fn.name === "sample") {
            actorEvaluations++;
            return new Promise(() => {});
          }
          if (actorIndex === 0) actorEvaluations++;
          if (arg?.path === "/servers/server-control" && arg.method === "GET") {
            return {
              status: 200,
              body: {
                id: "server-control",
                channels: [{ id: "text-control", kind: "text" }],
              },
            };
          }
          if (arg?.path?.startsWith("/servers/")) return { status: 204 };
          if (arg?.path === "/auth/logout") return { status: 200 };
          return { status: 200, body: { user: { id: "owned-control" } } };
        },
        close: async () => {
          actions.push("quarantined-page-close");
          if (brokenClose === "pending") return new Promise(() => {});
          if (brokenClose) throw new Error("PRIVATE-close-error");
          pageClosed = true;
        },
      };
      return {
        addInitScript: async () => {},
        newPage: async () => page,
        close: async () => {
          actions.push("cleanup-context-close");
          if (actorIndex === 0 && actorClose === "rejected") {
            pageClosed = true;
            throw new Error("PRIVATE-context-close-rejected");
          }
          if (brokenClose === "pending") return new Promise(() => {});
          if (brokenClose) throw new Error("PRIVATE-close-error");
          pageClosed = true;
        },
      };
    },
    close: async () => {
      actions.push("browser-close");
      if (brokenClose === "pending") return new Promise(() => {});
      if (brokenClose) throw new Error("PRIVATE-close-error");
    },
  };
  const process = {
    env: { GELABBER_E2E_REPORT: "/tmp/native-finish-control.json" },
    exitCode: 0,
    on() {},
  };
  const mocks = {
    playwright: {
      chromium: { launch: async () => browser },
      firefox: { launch: async () => browser },
    },
    "./probe.mjs": { instrument() {}, sample() {} },
    "./native-evaluate.mjs": {
      ...native,
      // Shorten only the infrastructure control's waits. Execute the actual
      // deadline helper; the committed harness retains its 5/12s budgets.
      deadlineProbe: (probe, deadline) =>
        native.deadlineProbe(probe, Math.min(deadline, Date.now() + 25)),
    },
    "./browser-options.mjs": options,
    "node:child_process": {
      execFileSync: (_cmd, args) =>
        args.includes("rev-parse") ? "a".repeat(40) : "",
    },
    "node:fs/promises": {
      readdir: async () => ["probe.mjs"],
      readFile: async () => "redacted-source-control",
      mkdir: async () => {},
      writeFile: async (path, body) => {
        if (!path.endsWith(".checkpoint.json")) {
          actions.push("report-write");
          reports.push(JSON.parse(body));
        }
      },
    },
  };
  const context = vm.createContext({ URL, process, console: { log() {} } });
  const source = new vm.SourceTextModule(
    await readFile(new URL("./harness.mjs", import.meta.url), "utf8"),
    {
      context,
      initializeImportMeta: (meta) => {
        meta.url = new URL("./harness.mjs", import.meta.url).href;
      },
    },
  );
  await source.link(async (name) => {
    const exports = mocks[name] ?? (await import(name));
    return new vm.SyntheticModule(
      Object.keys(exports),
      function () {
        for (const [k, v] of Object.entries(exports)) this.setExport(k, v);
      },
      { context },
    );
  });
  await source.evaluate();
  const h = await source.namespace.startHarness();
  const f = await h.fixture();
  const before = actorEvaluations;
  const failure = await h.run("native-interface-control", [], () =>
    actorClose
      ? actorClose === "external-page"
        ? f.owner.page.close()
        : f.owner.context.close()
      : stopRejected
        ? Promise.resolve({ control: "healthy-before-stop" })
        : native.nativeEvaluate(f.owner, function sample() {}, undefined, 2),
  );
  assert.equal(
    failure.status,
    actorClose === "rejected"
      ? "FAIL"
      : actorClose || stopRejected
        ? "PASS"
        : "FAIL",
  );
  if (!actorClose && !stopRejected)
    assert.equal(failure.classification, "test-error");
  const evaluatedAfterTimeout = actorEvaluations;
  await h.finish();
  if (!actorClose && !stopRejected)
    assert.equal(actorEvaluations, evaluatedAfterTimeout);
  return { actions, reports, process, before, evaluatedAfterTimeout };
}
test("actual finish skips only actor page whose owned context close completed successfully", async () => {
  const r = await finishProbe(false, false, "owned-context");
  assert.ok(!r.actions.includes("closed-actor-evaluate-attempt"));
  assert.equal(r.process.exitCode, 0);
  assert.equal(r.reports[0].gate.passed, true);
  assert.equal(r.reports[0].cleanup[0].status, 204);
  assert.equal(r.actions.at(-1), "report-write");
});
for (const mode of ["external-page", "rejected"]) {
  test(`actual finish keeps unexpected closed page or rejected owned close red ${mode}`, async () => {
    const r = await finishProbe(false, false, mode);
    assert.ok(r.actions.includes("closed-actor-evaluate-attempt"));
    assert.equal(r.process.exitCode, 1);
    assert.equal(r.reports[0].gate.passed, false);
    assert.ok(
      r.reports[0].results.some(
        (row) => row.id === "native-stop-evaluate" && row.status === "FAIL",
      ),
    );
    assert.equal(r.actions.at(-1), "report-write");
    assert.ok(!JSON.stringify(r).includes("PRIVATE"));
  });
}
test("actual harness InterfaceTimeout -> finish never reevaluates quarantined page and reaches cleanup/report", async () => {
  const r = await finishProbe();
  assert.equal(r.evaluatedAfterTimeout, r.before + 1);
  assert.ok(r.actions.includes("quarantined-page-close"));
  assert.ok(r.actions.includes("cleanup-context-close"));
  assert.equal(r.actions.at(-1), "report-write");
  assert.equal(r.process.exitCode, 1);
  assert.equal(r.reports[0].gate.passed, false);
  assert.equal(r.reports[0].cleanup[0].status, 204);
});
test("actual healthy stop rejection is redacted FAIL/Exit1 while all later cleanup/report actions continue", async () => {
  const r = await finishProbe(false, true);
  assert.ok(r.actions.includes("healthy-stop-evaluate-rejected"));
  assert.ok(r.actions.includes("cleanup-context-close"));
  assert.ok(r.actions.includes("browser-close"));
  assert.equal(r.actions.at(-1), "report-write");
  assert.equal(r.reports[0].cleanup[0].status, 204);
  assert.equal(r.process.exitCode, 1);
  assert.equal(r.reports[0].gate.passed, false);
  assert.ok(
    r.reports[0].results.some(
      (row) =>
        row.id === "native-stop-evaluate" &&
        row.status === "FAIL" &&
        row.classification === "test-error" &&
        row.reason === "native-stop-evaluate-rejected",
    ),
  );
  assert.ok(!JSON.stringify(r).includes("PRIVATE-native-close-error"));
});
test("actual harness broken page/context/browser closes stay red and cannot prevent later cleanup/report", async () => {
  const r = await finishProbe(true);
  assert.ok(r.actions.includes("cleanup-context-close"));
  assert.ok(r.actions.includes("browser-close"));
  assert.equal(r.actions.at(-1), "report-write");
  assert.equal(r.process.exitCode, 1);
  assert.equal(r.reports[0].gate.passed, false);
  assert.ok(
    r.reports[0].results.some(
      (row) => row.id === "cleanup-browser-close" && row.status === "FAIL",
    ),
  );
  assert.ok(!JSON.stringify(r).includes("PRIVATE-close-error"));
});
test("actual harness pending close paths time out visibly and still write the report", async () => {
  const r = await finishProbe("pending");
  assert.equal(r.actions.at(-1), "report-write");
  assert.equal(r.process.exitCode, 1);
  assert.equal(r.reports[0].cleanup[0].status, 204);
  for (const id of [
    "native-abort-close",
    "cleanup-context-close",
    "cleanup-browser-close",
  ])
    assert.ok(
      r.reports[0].results.some(
        (row) => row.id === id && row.status === "FAIL",
      ),
    );
  assert.equal(r.reports[0].gate.passed, false);
});

// GEL-453: the actual harness with a fake browser. `page` overrides parts of
// the fake page; `process` is the harness's own.
async function harnessProbe({ page: overrides = {}, process }) {
  const locator = {
    fill: async () => {},
    click: async () => {},
    first() {
      return this;
    },
    // Only the channel dialog's "Erstellen" takes the override.
    getByRole(role, { name } = {}) {
      if (role === "dialog") return { ...this, dialog: name };
      return this.dialog === "Kanal erstellen" &&
        name === "Erstellen" &&
        overrides.createClick
        ? { ...this, click: overrides.createClick }
        : this;
    },
    getByLabel() {
      return this;
    },
    getByText() {
      return this;
    },
    inputValue: async () => "http://127.0.0.1:5174/invite/control",
  };
  const browser = {
    version: () => "155.0",
    newContext: async () => ({
      addInitScript: async () => {},
      newPage: async () => ({
        goto: async () => {},
        waitForURL: async () => {},
        waitForFunction: async () => {},
        url: () => "http://127.0.0.1:5174/s/server-control/c/text-control",
        setDefaultTimeout() {},
        isClosed: () => false,
        getByRole: (role, options) => locator.getByRole(role, options),
        getByLabel: () => locator,
        locator: () => locator,
        waitForResponse: async () => ({
          status: () => 200,
          json: async () => ({ id: "voice-control" }),
        }),
        evaluate: async (_fn, arg) =>
          arg?.path === "/servers/server-control"
            ? {
                status: 200,
                body: { channels: [{ id: "text-control", kind: "text" }] },
              }
            : { status: 200, body: { user: { id: "owned-control" } } },
        close: async () => {},
        ...overrides,
      }),
      close: async () => {},
    }),
    close: async () => {},
  };
  const mocks = {
    playwright: {
      chromium: { launch: async () => browser },
      firefox: { launch: async () => browser },
    },
    "./probe.mjs": { instrument() {}, sample() {} },
    "./native-evaluate.mjs": native,
    "./browser-options.mjs": options,
    "node:child_process": {
      execFileSync: (_cmd, args) =>
        args.includes("rev-parse") ? "a".repeat(40) : "",
    },
    "node:fs/promises": {
      readdir: async () => [],
      readFile: async () => "",
      mkdir: async () => {},
      writeFile: async () => {},
    },
  };
  const harnessUrl = new URL("./harness.mjs", import.meta.url);
  const context = vm.createContext({ URL, process, console: { log() {} } });
  const source = new vm.SourceTextModule(await readFile(harnessUrl, "utf8"), {
    context,
    identifier: harnessUrl.href,
    initializeImportMeta: (meta) => {
      meta.url = harnessUrl.href;
    },
  });
  await source.link(async (name) => {
    const exports = mocks[name] ?? (await import(name));
    return new vm.SyntheticModule(
      Object.keys(exports),
      function () {
        for (const [k, v] of Object.entries(exports)) this.setExport(k, v);
      },
      { context },
    );
  });
  await source.evaluate();
  return source.namespace.startHarness();
}
const harnessProcess = () => {
  const listeners = {};
  return {
    env: { GELABBER_E2E_REPORT: "/tmp/stray-rejection-control.json" },
    exitCode: 0,
    listeners,
    on(event, listener) {
      listeners[event] = listener;
    },
  };
};
test("a fixture whose channel click fails leaves no response waiter unheard", async () => {
  const unheard = [];
  const listen = (reason) => unheard.push(reason);
  globalThis.process.on("unhandledRejection", listen);
  try {
    let expire;
    const h = await harnessProbe({
      process: harnessProcess(),
      page: {
        waitForResponse: () =>
          new Promise((_, reject) => {
            expire = reject;
          }),
        createClick: async () => {
          throw new Error("create-click-failed");
        },
      },
    });
    await assert.rejects(h.fixture(), /create-click-failed/);
    // The waiter's own deadline passes after the fixture already failed.
    expire(new Error("page.waitForResponse: Timeout 12000ms exceeded."));
    await new Promise((done) => setImmediate(done));
  } finally {
    globalThis.process.off("unhandledRejection", listen);
  }
  assert.deepEqual(unheard, []);
});
test("a stray rejection fails by name and the run goes on", async () => {
  const process = harnessProcess();
  const h = await harnessProbe({ process });
  const before = await h.run("first-control", [], async () => ({}));
  assert.equal(before.status, "PASS");
  // A promise first-control left behind rejects later, unheard.
  const stray = new Error(
    'page.waitForResponse: Timeout 12000ms exceeded while waiting for event "response" PRIVATE',
  );
  stray.name = "TimeoutError";
  stray.stack = `${stray.message}\n    at file:///w/web/scripts/e2e/core.mjs:55:41`;
  assert.equal(typeof process.listeners.unhandledRejection, "function");
  process.listeners.unhandledRejection(stray, Promise.resolve());
  const after = await h.run("second-control", [], async () => ({}));
  assert.equal(after.status, "PASS");
  const row = h.report.results.find((r) => r.id === "stray-rejection");
  assert.equal(row.status, "FAIL");
  assert.equal(row.reason, "browser-deadline");
  assert.equal(row.during, "first-control");
  assert.equal(row.site, "core.mjs:55:41");
  assert.equal(process.exitCode, 1);
  assert.ok(!JSON.stringify(h.report.results).includes("PRIVATE"));
});
