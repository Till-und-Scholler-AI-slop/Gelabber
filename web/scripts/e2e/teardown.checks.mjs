/* global URL, queueMicrotask, process */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import vm from "node:vm";
import { promisify } from "node:util";
import { attemptAll, closeOwnedApi, TeardownFailure } from "./teardown.mjs";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as pause } from "node:timers/promises";
import { recordOwned, removeLeftover } from "./owned-process.mjs";
import { failureSite, safeError, safeTarget } from "./harness.mjs";

const secret = "PRIVATE-cookie-signed-URL-password";
async function executeSource(filename, mocks, globals = {}) {
  const context = vm.createContext(globals);
  const cache = new Map();
  async function load(
    specifier,
    reference = new URL(filename, import.meta.url),
  ) {
    const key =
      specifier === "./teardown.mjs"
        ? new URL(specifier, reference).href
        : specifier;
    if (cache.has(key)) return cache.get(key);
    let module;
    if (specifier === "./teardown.mjs") {
      module = new vm.SourceTextModule(await readFile(new URL(key), "utf8"), {
        context,
        identifier: key,
      });
    } else {
      const exports = mocks[specifier] ?? (await import(specifier));
      module = new vm.SyntheticModule(
        Object.keys(exports),
        function () {
          for (const [k, v] of Object.entries(exports)) this.setExport(k, v);
        },
        { context, identifier: key },
      );
    }
    cache.set(key, module);
    await module.link((name, from) =>
      load(name, new URL(from.identifier, import.meta.url)),
    );
    return module;
  }
  const path = new URL(filename, import.meta.url);
  const module = new vm.SourceTextModule(await readFile(path, "utf8"), {
    context,
    identifier: path.href,
    initializeImportMeta: (meta) => {
      meta.url = path.href;
    },
    importModuleDynamically: async (name) => {
      const child = await load(name, path);
      await child.evaluate();
      return child;
    },
  });
  await module.link((name) => load(name, path));
  await module.evaluate();
  return module.namespace;
}

async function runnerProbe({
  fail = [],
  startup = false,
  reportFailure = false,
  media = false,
  extraEnv = {},
  hashChange = false,
  returnedMediaOrigin = "http://127.0.0.1:18087",
  strayAtStartup = false,
  harnessListening = false,
} = {}) {
  const acquired = [],
    hashReads = new Map();
  const calls = [],
    records = [],
    messages = [];
  let routedMedia;
  const action = async (name) => {
    calls.push(name);
    if (fail.includes(name)) throw new Error(secret);
  };
  const proxy = (name) => ({ restore: () => action(`${name}.restore`) });
  const runtime = {
    origin: "http://127.0.0.1:18086",
    manifest: { sourceSha: "a".repeat(40) },
    redis: proxy("redis"),
    storage: proxy("storage"),
    database: proxy("database"),
    close: () => action("runtime.close"),
    resumeApi: () => action("api.resume"),
  };
  const listeners = {};
  const fakeProcess = {
    exitCode: 0,
    listeners,
    on(event, listener) {
      (listeners[event] ??= []).push(listener);
    },
    listenerCount: (event) => listeners[event]?.length ?? 0,
    env: {
      GELABBER_E2E_FAULT_ENV: "/tmp/fixture.env",
      GELABBER_E2E_API_MANIFEST: "/tmp/fixture-manifest.json",
      GELABBER_E2E_WEB_SNAPSHOT: "/tmp/fixture-web",
      ...extraEnv,
      ...(media
        ? {
            GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media.env",
            GELABBER_E2E_MEDIA_MANIFEST: "/tmp/media-manifest.json",
          }
        : {}),
    },
  };
  await executeSource(
    "./fault-run.mjs",
    {
      vite: {
        createServer: async (options) => {
          routedMedia = options.server.proxy["/media"].target;
          return {
            listen: async () => {
              if (startup) throw new Error(secret);
            },
            close: () => action("web.close"),
          };
        },
      },
      "./harness.mjs": { failureSite, safeError, safeTarget },
      "./fault-runtime.mjs": {
        startFaultApi: async () => {
          acquired.push("api");
          if (strayAtStartup) {
            // The harness's own listener, once startHarness has run.
            if (harnessListening)
              fakeProcess.on("unhandledRejection", () => {});
            const stray = new Error(`connect ECONNREFUSED ${secret}`);
            stray.stack = `${stray.message}\n    at file:///w/web/scripts/e2e/faults.mjs:12:3`;
            for (const listener of listeners.unhandledRejection ?? [])
              listener(stray, Promise.resolve());
          }
          return runtime;
        },
        useFaultRuntime: (x) => {
          calls.push(x ? "runtime.set" : "runtime.clear");
        },
      },
      "./media-runtime.mjs": {
        startFaultMedia: async () => {
          acquired.push("media");
          return {
            origin: returnedMediaOrigin,
            manifest: { sourceSha: "b".repeat(40) },
            close: () => action("media.close"),
          };
        },
      },
      "./run.mjs": {},
      "node:fs/promises": {
        readFile: async (path) => {
          const count = (hashReads.get(path) ?? 0) + 1;
          hashReads.set(path, count);
          return hashChange && count > 1
            ? "changed-private-fixture"
            : "private-fixture";
        },
        mkdtemp: async () => "/tmp/fixture-cache",
        rm: () => action("cache.remove"),
        mkdir: async () => {},
        writeFile: async (path, body) => {
          records.push({ path, body: JSON.parse(body) });
          if (reportFailure) throw new Error(secret);
        },
      },
    },
    { process: fakeProcess, console: { error: (msg) => messages.push(msg) } },
  );
  return {
    calls,
    acquired,
    records,
    messages,
    routedMedia,
    exitCode: fakeProcess.exitCode,
  };
}

test("exact runner attempts API/cache cleanup after web.close rejects and records only redacted FAIL", async () => {
  const result = await runnerProbe({ fail: ["web.close"] });
  assert.ok(
    result.calls.includes("runtime.close") &&
      result.calls.includes("cache.remove") &&
      result.calls.includes("runtime.clear"),
  );
  assert.equal(result.exitCode, 1);
  assert.deepEqual(result.records.at(-1).body.failedSteps, ["web-close"]);
  assert.equal(result.records.at(-1).body.status, "FAIL");
  assert.equal(result.records.at(-1).body.completeAcceptance, false);
  assert.ok(!JSON.stringify(result).includes(secret));
});

const ownAdapterEnv = {
  GELABBER_E2E_BROWSER: "firefox",
  GELABBER_E2E_NETWORK: "relay",
  GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "true",
  GELABBER_E2E_SFU_LOOPBACK_ICE: "true",
  GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media-fault-loopback-control.env",
  GELABBER_E2E_MEDIA_MANIFEST: "/tmp/media-manifest.json",
};
test("exact own-SFU runner rejects mismatched media override before API/SFU acquisition with both adapters on or off", async () => {
  for (const adapter of [false, true]) {
    const result = await runnerProbe({
      extraEnv: {
        ...(adapter
          ? ownAdapterEnv
          : {
              GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media-fault.env",
              GELABBER_E2E_MEDIA_MANIFEST: "/tmp/media-manifest.json",
            }),
        GELABBER_E2E_MEDIA_ORIGIN: "http://127.0.0.1:18088",
      },
    });
    assert.deepEqual(result.acquired, []);
    assert.equal(result.routedMedia, undefined);
    assert.equal(result.exitCode, 1);
    assert.equal(result.records[0].body.status, "BLOCKED");
    assert.ok(!JSON.stringify(result).includes(secret));
  }
});
test("exact own-SFU runner routes only to returned attested origin with default or equal explicit override", async () => {
  for (const override of [undefined, "http://127.0.0.1:18087"]) {
    const result = await runnerProbe({
      extraEnv: {
        ...ownAdapterEnv,
        ...(override ? { GELABBER_E2E_MEDIA_ORIGIN: override } : {}),
      },
    });
    assert.equal(result.routedMedia, "http://127.0.0.1:18087");
    assert.equal(result.exitCode, 0);
    assert.equal(result.records.at(-1).body.status, "PASS");
  }
});
test("unexpected acquired SFU origin cannot reach Vite/run; all acquired resources still close", async () => {
  const result = await runnerProbe({
    extraEnv: ownAdapterEnv,
    returnedMediaOrigin: "http://127.0.0.1:18088",
  });
  assert.equal(result.exitCode, 1);
  assert.equal(result.routedMedia, undefined);
  assert.ok(result.calls.includes("media.close"));
  assert.ok(result.calls.includes("runtime.close"));
});
test("exact runner attempts every restore/close even with multiple failures and failed artifact writes", async () => {
  const result = await runnerProbe({
    fail: ["redis.restore", "web.close", "runtime.close", "cache.remove"],
    reportFailure: true,
  });
  for (const name of [
    "storage.restore",
    "database.restore",
    "web.close",
    "runtime.close",
    "runtime.clear",
    "cache.remove",
  ])
    assert.ok(result.calls.includes(name));
  assert.equal(result.exitCode, 1);
  assert.ok(result.messages.includes("FAIL owned-teardown-report-write-error"));
  assert.ok(!JSON.stringify(result).includes(secret));
});
test("exact runner startup failure retains nonzero and still attempts every acquired resource", async () => {
  const result = await runnerProbe({ startup: true, fail: ["web.close"] });
  assert.equal(result.exitCode, 1);
  assert.equal(result.records[0].body.status, "BLOCKED");
  assert.ok(
    result.calls.includes("runtime.close") &&
      result.calls.includes("cache.remove"),
  );
});

class FakeChild extends EventEmitter {
  constructor(calls, role) {
    super();
    this.pid = 1;
    this.exitCode = null;
    this.signalCode = null;
    this.calls = calls;
    this.role = role;
    this.stdout = new EventEmitter();
    this.stdin = new EventEmitter();
  }
  kill(signal) {
    this.calls.push(`${this.role}.${signal}`);
    this.signalCode = signal;
    this.emit("exit", null, signal);
  }
}
test("API teardown continues across failed lock, restore, close and log stages without mutating proxy order", async () => {
  const calls = [];
  const proxies = [0, 1, 2].map((i) => ({
    restore() {
      calls.push(`restore${i}`);
      if (i === 0) throw new Error(secret);
    },
    async close() {
      calls.push(`close${i}`);
      if (i === 2) throw new Error(secret);
    },
  }));
  const first = proxies[0];
  const child = new FakeChild(calls, "api");
  await assert.rejects(
    closeOwnedApi({
      locks: new Set([
        async () => {
          calls.push("lock0");
          throw new Error(secret);
        },
        async () => {
          calls.push("lock1");
        },
      ]),
      proxies,
      child,
      logfile: {
        async close() {
          calls.push("log");
          throw new Error(secret);
        },
      },
    }),
    (e) => e instanceof TeardownFailure && !JSON.stringify(e).includes(secret),
  );
  assert.deepEqual(calls, [
    "lock0",
    "lock1",
    "restore0",
    "restore1",
    "restore2",
    "api.SIGTERM",
    "close2",
    "close1",
    "close0",
    "log",
  ]);
  assert.equal(proxies[0], first);
});
test("exact startFaultApi.close releases remaining proxies/log and stops the own locker after lock pipe failure", async () => {
  const calls = [],
    binary = "fixture-binary";
  const manifest = {
    sourceSha: "a".repeat(40),
    sha256: createHash("sha256").update(binary).digest("hex"),
    binaryPath: "/tmp/fixture-api",
  };
  const children = [];
  const sources = {
    "/tmp/fixture.env":
      'DATABASE_URL="postgres://fixture@127.0.0.1:5432/gelabber_fault_abcd"\nREDIS_URL="redis://127.0.0.1:6379"\nMINIO_ENDPOINT="http://127.0.0.1:9000"\nMINIO_ROOT_USER="fixture"\nMINIO_ROOT_PASSWORD="private-fixture"\nMINIO_BUCKET="gb-fault-abcd"\n',
    "/tmp/fixture-manifest.json": JSON.stringify(manifest),
    "/tmp/fixture-api": binary,
  };
  const portProbe = new EventEmitter();
  portProbe.listen = () => {
    queueMicrotask(() => portProbe.emit("listening"));
  };
  portProbe.close = (done) => done();
  let index = 0;
  const execFileMock = () => {};
  execFileMock[promisify.custom] = async (_cmd, args) => ({
    stdout: args.at(-1).includes("pg_stat_activity") ? "0\n" : "1\n",
    stderr: "",
  });
  const proxy = () => {
    const id = index++;
    return {
      port: 10000 + id,
      restore() {
        calls.push(`restore${id}`);
        if (id === 0) throw new Error(secret);
      },
      async close() {
        calls.push(`close${id}`);
        if (id === 1) throw new Error(secret);
      },
    };
  };
  const source = await executeSource(
    "./fault-runtime.mjs",
    {
      "node:fs/promises": {
        stat: async () => ({ mode: 0o100600 }),
        readFile: async (path) => sources[path],
        open: async () => ({
          fd: 9,
          async close() {
            calls.push("log");
          },
        }),
        readdir: async () => [],
        readlink: async () => "",
      },
      "node:child_process": {
        execFile: execFileMock,
        spawn(path) {
          const child = new FakeChild(
            calls,
            path === "psql" ? "locker" : "api",
          );
          children.push(child);
          child.stdin.write = () => child.stdout.emit("data", "E2E_LOCKED\n");
          child.stdin.end = () => {
            throw new Error(secret);
          };
          return child;
        },
      },
      "node:net": { createServer: () => portProbe },
      "node:timers/promises": { setTimeout: async () => {} },
      "./faults.mjs": {
        tcpFaultProxy: async () => proxy(),
        storageFaultProxy: async () => proxy(),
      },
      "./s3-object.mjs": { objectStatus: async () => 404 },
      "./redis-command.mjs": {
        deleteTopicKeys: async () => 4,
      },
      "./owned-process.mjs": {
        removeLeftover: async () => {
          calls.push("owned.leftover");
          return null;
        },
        recordOwned: async () => calls.push("owned.record"),
        forgetOwned: async () => calls.push("owned.forget"),
        killOnExit: () => {
          calls.push("owned.hook");
          return () => calls.push("owned.unhook");
        },
      },
    },
    { process: { env: {} }, URL, fetch: async () => ({ ok: true }) },
  );
  const runtime = await source.startFaultApi({
    envPath: "/tmp/fixture.env",
    manifestPath: "/tmp/fixture-manifest.json",
  });
  await runtime.lockOwnedServer(
    "11111111-1111-4111-8111-111111111111",
    "22222222-2222-4222-8222-222222222222",
  );
  let failure;
  try {
    await runtime.close();
  } catch (error) {
    failure = error;
  }
  assert.equal(failure?.name, "TeardownFailure");
  for (const name of [
    "locker.SIGTERM",
    "api.SIGTERM",
    "restore0",
    "restore1",
    "restore2",
    "close2",
    "close1",
    "close0",
    "log",
  ])
    assert.ok(calls.includes(name), name);
  assert.ok(children.every((c) => c.signalCode === "SIGTERM"));
  assert.ok(!JSON.stringify(failure).includes(secret));
  // An earlier runner's leftover goes first; the new API stays tied to this
  // runner (exit hook, PID file) while its teardown failed (GEL-457).
  assert.deepEqual(
    calls.filter((name) => name.startsWith("owned.")),
    ["owned.leftover", "owned.hook", "owned.record"],
  );
});
test("successful actions leave gate unchanged and preserve sequential teardown order", async () => {
  const result = await runnerProbe();
  assert.equal(result.exitCode, 0);
  assert.deepEqual(result.records, []);
  const calls = [];
  assert.deepEqual(
    await attemptAll([
      ["first", async () => calls.push(1)],
      ["second", () => calls.push(2)],
    ]),
    [],
  );
  assert.deepEqual(calls, [1, 2]);
});

test("media close and API resume failures cannot skip remaining owned runtime/cache cleanup", async () => {
  const result = await runnerProbe({
    media: true,
    fail: ["api.resume", "media.close"],
  });
  assert.equal(result.exitCode, 1);
  for (const name of [
    "api.resume",
    "web.close",
    "media.close",
    "runtime.close",
    "runtime.clear",
    "cache.remove",
  ])
    assert.ok(result.calls.includes(name));
  assert.deepEqual(result.records.at(-1).body.failedSteps, [
    "api-resume",
    "media-runtime-close",
  ]);
  assert.ok(!JSON.stringify(result).includes(secret));
});

test("exact runner rejects invalid adapter combinations before acquiring API or SFU resources", async () => {
  for (const extraEnv of [
    { GELABBER_E2E_SFU_LOOPBACK_ICE: "true" },
    { GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "bad" },
    {
      GELABBER_E2E_BROWSER: "firefox",
      GELABBER_E2E_NETWORK: "relay",
      GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "true",
      GELABBER_E2E_SFU_LOOPBACK_ICE: "true",
      GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media-fault.env",
    },
  ]) {
    const result = await runnerProbe({ extraEnv });
    assert.deepEqual(result.acquired, []);
    assert.equal(result.exitCode, 1);
    assert.equal(result.records[0].body.status, "BLOCKED");
  }
});
test("exact adapter runner records equal baseline hashes and makes changed baseline or report failure nonzero after all cleanup", async () => {
  const extraEnv = {
    GELABBER_E2E_BROWSER: "firefox",
    GELABBER_E2E_NETWORK: "relay",
    GELABBER_E2E_FIREFOX_LOOPBACK_ICE: "true",
    GELABBER_E2E_SFU_LOOPBACK_ICE: "true",
    GELABBER_E2E_MEDIA_FAULT_ENV: "/tmp/media-fault-loopback-control.env",
    GELABBER_E2E_MEDIA_MANIFEST: "/tmp/media-manifest.json",
  };
  for (const hashChange of [false, true]) {
    const result = await runnerProbe({ extraEnv, hashChange });
    assert.equal(result.exitCode, hashChange ? 1 : 0);
    assert.equal(result.records.at(-1).body.baselineUnchanged, !hashChange);
    assert.ok(result.calls.includes("runtime.close"));
    assert.ok(result.calls.includes("cache.remove"));
    assert.ok(!JSON.stringify(result).includes(secret));
  }
  assert.equal(
    (await runnerProbe({ extraEnv, reportFailure: true })).exitCode,
    1,
  );
});
test("a stray rejection during start-up fails by name and the runner still tears down", async () => {
  const result = await runnerProbe({ strayAtStartup: true });
  assert.equal(result.exitCode, 1);
  const record = result.records.find((r) =>
    r.path.endsWith(".startup-stray.json"),
  );
  assert.equal(record.body.status, "FAIL");
  assert.equal(record.body.results[0].id, "stray-rejection");
  assert.equal(record.body.results[0].during, "startup");
  assert.equal(record.body.results[0].site, "faults.mjs:12:3");
  assert.ok(
    result.messages.some((m) =>
      m.startsWith("FAIL stray-rejection during start-up"),
    ),
  );
  for (const name of ["web.close", "runtime.close", "cache.remove"])
    assert.ok(result.calls.includes(name), name);
  assert.ok(!JSON.stringify(result).includes(secret));
  // Once the harness listens, its own row records a stray, not this one.
  const later = await runnerProbe({
    strayAtStartup: true,
    harnessListening: true,
  });
  assert.equal(later.exitCode, 0);
  assert.ok(!later.records.some((r) => r.path.endsWith(".startup-stray.json")));
});

// GEL-457 with real processes: a stand-in for the API or SFU that runs until
// it is killed.
function sleeper(t) {
  const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], {
    stdio: "ignore",
  });
  t.after(() => {
    if (child.exitCode === null && child.signalCode === null)
      child.kill("SIGKILL");
  });
  return child;
}
// Running, as opposed to gone or a zombie, from /proc/<pid>/stat.
async function running(pid) {
  const stat = await readFile(`/proc/${pid}/stat`, "utf8").catch(() => null);
  return Boolean(stat) && stat.slice(stat.lastIndexOf(")") + 2)[0] !== "Z";
}
async function untilGone(pid, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while ((await running(pid)) && Date.now() < deadline) await pause(50);
  return !(await running(pid));
}
test("a runner that crashes takes its owned child with it", async (t) => {
  const module = new URL("./owned-process.mjs", import.meta.url).href;
  const runner = spawn(
    process.execPath,
    [
      "--input-type=module",
      "-e",
      `import { spawn } from "node:child_process";
       import { killOnExit } from ${JSON.stringify(module)};
       const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1e6)"], { stdio: "ignore" });
       killOnExit(child);
       console.log(child.pid);
       // A rejection nobody awaits ends this runner without any teardown.
       Promise.reject(new Error("runner-crash"));`,
    ],
    { stdio: ["ignore", "pipe", "ignore"] },
  );
  let out = "";
  runner.stdout.on("data", (data) => (out += data));
  const [code] = await once(runner, "exit");
  const pid = Number(out.trim());
  t.after(() => {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Already gone, as it should be.
    }
  });
  assert.notEqual(code, 0);
  assert.ok(Number.isInteger(pid) && pid > 1, out);
  assert.ok(await untilGone(pid), `owned child ${pid} outlived its runner`);
});
test("the next start removes a recorded leftover and nothing else", async (t) => {
  const dir = await mkdtemp(join(tmpdir(), "gelabber-owned-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const pidFile = join(dir, "api.pid");
  assert.equal(await removeLeftover(pidFile), null);
  // A runner killed with SIGKILL left this one behind.
  const leftover = sleeper(t);
  await once(leftover, "spawn");
  await recordOwned(pidFile, leftover.pid, process.execPath);
  const exited = once(leftover, "exit");
  assert.equal(await removeLeftover(pidFile), leftover.pid);
  assert.deepEqual(await exited, [null, "SIGKILL"]);
  assert.equal(await removeLeftover(pidFile), null);
  // The PID now belongs to another process (another start time), or runs
  // another binary: neither is touched.
  const other = sleeper(t);
  await once(other, "spawn");
  await recordOwned(pidFile, other.pid, process.execPath);
  const record = JSON.parse(await readFile(pidFile, "utf8"));
  for (const changed of [
    { ...record, started: String(Number(record.started) + 1) },
    { ...record, exe: "/opt/gelabber-api" },
  ]) {
    await writeFile(pidFile, JSON.stringify(changed));
    assert.equal(await removeLeftover(pidFile), null);
    assert.ok(await running(other.pid));
  }
});
