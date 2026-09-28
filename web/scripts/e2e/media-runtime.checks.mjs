/* global URL, queueMicrotask */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import vm from "node:vm";
import * as teardown from "./teardown.mjs";

async function mediaProbe({
  failStop = false,
  foreign = false,
  loopback = false,
  optIn = false,
  portMax = false,
} = {}) {
  const calls = [],
    binary = "owned-media-control",
    secret = "PRIVATE-media-env-error";
  const manifest = {
    sourceSha: "a".repeat(40),
    binaryPath: "/tmp/media-control",
    sha256: createHash("sha256").update(binary).digest("hex"),
  };
  const port = new EventEmitter();
  port.listen = () => queueMicrotask(() => port.emit("listening"));
  port.close = (fn) => fn();
  const child = new EventEmitter();
  child.pid = 1;
  child.exitCode = null;
  child.signalCode = null;
  child.kill = (signal) => {
    calls.push(signal);
    if (failStop && signal === "SIGTERM") throw new Error(secret);
    child.signalCode = signal;
    child.emit("exit", null, signal);
    return true;
  };
  let launchedEnv;
  const mocks = {
    "./teardown.mjs": {
      ...teardown,
      terminateOwnedChild: (child) => teardown.terminateOwnedChild(child, 0),
    },
    "node:child_process": {
      spawn: (_path, _args, options) => {
        launchedEnv = options.env;
        calls.push("spawn");
        return child;
      },
    },
    "node:fs/promises": {
      stat: async () => ({ mode: 0o100600 }),
      readFile: async (path) =>
        path === "/tmp/media.env"
          ? `MEDIA_ADDR=${foreign ? "127.0.0.1:8087" : "127.0.0.1:18087"}\nREDIS_URL=redis://127.0.0.1:6379\nMEDIA_ICE_BIND=${loopback ? "127.0.0.1" : "192.0.2.1"}:0\nMEDIA_ADVERTISED_IP=${loopback ? "127.0.0.1" : "192.0.2.1"}\n${portMax ? "MEDIA_ICE_PORT_MAX=20000\n" : ""}`
          : path === "/proc/1/environ"
            ? Object.entries(launchedEnv)
                .map(([key, value]) => `${key}=${value}`)
                .join("\0")
            : path === "/tmp/manifest.json"
              ? JSON.stringify(manifest)
              : binary,
      open: async () => ({
        fd: 9,
        close: async () => {
          calls.push("log.close");
        },
      }),
    },
    "node:net": { createServer: () => port },
  };
  const context = vm.createContext({
    URL,
    process: { env: {} },
    fetch: async () => ({ ok: true }),
  });
  const source = new vm.SourceTextModule(
    await readFile(new URL("./media-runtime.mjs", import.meta.url), "utf8"),
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
  let failure;
  try {
    const runtime = await source.namespace.startFaultMedia({
      envPath: "/tmp/media.env",
      manifestPath: "/tmp/manifest.json",
      redisPort: 16386,
      loopbackIceAdapter: optIn,
    });
    await runtime.close();
  } catch (error) {
    failure = error;
  }
  return { calls, failure, secret };
}
test("own SFU child-stop failure still closes private log and exposes only controlled teardown steps", async () => {
  const { calls, failure, secret } = await mediaProbe({ failStop: true });
  assert.deepEqual(calls, ["spawn", "SIGTERM", "SIGKILL", "log.close"]);
  assert.equal(failure.name, "TeardownFailure");
  assert.deepEqual(failure.steps, ["media-child-stop", "child-sigterm"]);
  assert.ok(!JSON.stringify(failure).includes(secret));
});
test("own SFU refuses a shared port before spawn; successful owned teardown remains bounded and ordered", async () => {
  const foreign = await mediaProbe({ foreign: true });
  assert.equal(foreign.calls.length, 0);
  assert.ok(foreign.failure);
  const own = await mediaProbe();
  assert.equal(own.failure, undefined);
  assert.deepEqual(own.calls, ["spawn", "SIGTERM", "log.close"]);
});

test("SFU loopback is refused without opt-in, validates private control endpoints, and never inherits a port maximum", async () => {
  for (const options of [
    { loopback: true },
    { optIn: true },
    { loopback: true, optIn: true, portMax: true },
  ]) {
    const probe = await mediaProbe(options);
    assert.ok(probe.failure);
    assert.equal(probe.calls.length, 0);
  }
  const positive = await mediaProbe({ loopback: true, optIn: true });
  assert.equal(positive.failure, undefined);
  assert.deepEqual(positive.calls, ["spawn", "SIGTERM", "log.close"]);
});
