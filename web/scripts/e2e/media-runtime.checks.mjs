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
  logFails = false,
} = {}) {
  const calls = [],
    owned = [],
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
    "./owned-process.mjs": {
      clearLeftover: async () => {
        owned.push("leftover");
        return {};
      },
      heldByLeftover: (_leftover, error) => error,
      recordOwned: async (_file, pid) => owned.push(`record-${pid}`),
      forgetOwned: async () => owned.push("forget"),
      killOnExit: () => {
        owned.push("hook");
        return () => owned.push("unhook");
      },
    },
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
      open: async () => {
        if (logFails) throw new Error(secret);
        return {
          fd: 9,
          close: async () => {
            calls.push("log.close");
          },
        };
      },
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
  return { calls, owned, failure, secret };
}
test("own SFU child-stop failure still closes private log and exposes only controlled teardown steps", async () => {
  const { calls, owned, failure, secret } = await mediaProbe({
    failStop: true,
  });
  assert.deepEqual(calls, ["spawn", "SIGTERM", "SIGKILL", "log.close"]);
  assert.equal(failure.name, "TeardownFailure");
  // A failed teardown keeps the exit hook and the PID file (GEL-457).
  assert.deepEqual(owned, ["leftover", "hook", "record-1"]);
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
  // A leftover of an earlier runner goes before the port is probed; the new
  // SFU is tied to this runner until its teardown succeeded (GEL-457).
  assert.deepEqual(own.owned, [
    "leftover",
    "hook",
    "record-1",
    "unhook",
    "forget",
  ]);
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
test("an SFU start that fails before it records its child leaves the PID file alone", async () => {
  // Another run's fresh file is not this start's to delete (GEL-457).
  const { owned, failure } = await mediaProbe({ logFails: true });
  assert.ok(failure);
  assert.deepEqual(owned, ["leftover"]);
});
