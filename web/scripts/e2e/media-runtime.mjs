/* global process, fetch, URL */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { open, readFile, stat } from "node:fs/promises";
import { parseEnv } from "node:util";
import { setTimeout as pause } from "node:timers/promises";
import { createServer } from "node:net";
import { once } from "node:events";
import {
  attemptAll,
  terminateOwnedChild,
  TeardownFailure,
} from "./teardown.mjs";

export async function startFaultMedia({
  envPath,
  manifestPath,
  redisPort,
  loopbackIceAdapter = false,
}) {
  assert.equal(
    (await stat(envPath)).mode & 0o077,
    0,
    "Private media env required",
  );
  const env = parseEnv(await readFile(envPath, "utf8"));
  assert.equal(
    env.MEDIA_ADDR,
    "127.0.0.1:18087",
    "Only own reserved SFU port allowed",
  );
  const loopback =
    env.MEDIA_ICE_BIND?.startsWith("127.") ||
    env.MEDIA_ADVERTISED_IP?.startsWith("127.");
  assert.ok(
    !loopback || loopbackIceAdapter,
    "Loopback SFU requires explicit topology adapter",
  );
  if (loopbackIceAdapter) {
    assert.equal(env.MEDIA_ICE_BIND, "127.0.0.1:0");
    assert.equal(env.MEDIA_ADVERTISED_IP, "127.0.0.1");
    assert.equal(env.MEDIA_ICE_PORT_MAX, undefined);
    assert.equal(
      process.env.MEDIA_ICE_PORT_MAX,
      undefined,
      "Control must not inherit an ICE port maximum",
    );
  }
  const redis = new URL(env.REDIS_URL);
  assert.ok(["127.0.0.1", "localhost"].includes(redis.hostname));
  assert.equal(redis.protocol, "redis:");
  assert.ok(
    Number.isInteger(redisPort) && redisPort === 16386,
    "Own Redis proxy required",
  );
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.match(manifest.sourceSha, /^[a-f0-9]{40}$/);
  assert.match(manifest.sha256, /^[a-f0-9]{64}$/);
  assert.equal(
    createHash("sha256")
      .update(await readFile(manifest.binaryPath))
      .digest("hex"),
    manifest.sha256,
  );
  const probe = createServer();
  probe.listen({ host: "127.0.0.1", port: 18087, exclusive: true });
  await once(probe, "listening");
  await new Promise((r) => probe.close(r));
  let child, logfile;
  async function close() {
    const failed = await attemptAll([
      ["media-child-stop", () => terminateOwnedChild(child)],
      ["media-log-close", () => logfile?.close()],
    ]);
    if (failed.length) throw new TeardownFailure(failed);
  }
  try {
    logfile = await open("/tmp/gelabber-e2e-fault-media-18087.log", "w", 0o600);
    redis.hostname = "127.0.0.1";
    redis.port = String(redisPort);
    child = spawn(manifest.binaryPath, [], {
      env: { ...process.env, ...env, REDIS_URL: redis.href },
      stdio: ["ignore", logfile.fd, logfile.fd],
    });
    let spawnError = false;
    child.on("error", () => {
      spawnError = true;
    });
    const origin = "http://127.0.0.1:18087";
    const deadline = Date.now() + 30_000;
    let ready = false;
    do {
      assert.ok(
        !spawnError && child.exitCode === null && child.signalCode === null,
        "Owned SFU startup failed",
      );
      ready = await fetch(`${origin}/ready`)
        .then((r) => r.ok)
        .catch(() => false);
      if (ready) break;
      await pause(200);
    } while (Date.now() < deadline);
    assert.ok(ready, "Owned SFU readiness deadline");
    const running = Object.fromEntries(
      (await readFile(`/proc/${child.pid}/environ`, "utf8"))
        .split("\0")
        .filter(Boolean)
        .map((item) => {
          const at = item.indexOf("=");
          return [item.slice(0, at), item.slice(at + 1)];
        }),
    );
    assert.equal(
      running.MEDIA_ICE_BIND,
      env.MEDIA_ICE_BIND,
      "Actual own SFU ICE bind mismatch",
    );
    assert.equal(
      running.MEDIA_ADVERTISED_IP,
      env.MEDIA_ADVERTISED_IP,
      "Actual own SFU advertised category mismatch",
    );
    if (loopbackIceAdapter) assert.equal(running.MEDIA_ICE_PORT_MAX, undefined);
    assert.equal(
      createHash("sha256")
        .update(await readFile(`/proc/${child.pid}/exe`))
        .digest("hex"),
      manifest.sha256,
      "Actual own SFU binary mismatch",
    );
    return {
      origin,
      manifest,
      pid: child.pid,
      iceAdapter: {
        enabled: loopbackIceAdapter,
        actualProcessParametersMatchPrivateConfig: true,
        actualProcessBinaryMatchesManifest: true,
        name: loopbackIceAdapter
          ? "explicit-own-SFU-loopback-ICE-topology-control"
          : "approved-baseline",
        bindCategory: loopback ? "loopback-v4" : "non-loopback-local-address",
        ephemeralUdp: env.MEDIA_ICE_BIND?.endsWith(":0") === true,
        advertisedCategory: env.MEDIA_ADVERTISED_IP?.startsWith("127.")
          ? "loopback-v4"
          : "non-loopback-local-address",
        icePortMaximumAbsent:
          env.MEDIA_ICE_PORT_MAX === undefined &&
          process.env.MEDIA_ICE_PORT_MAX === undefined,
      },
      close,
    };
  } catch (error) {
    await close();
    throw error;
  }
}
