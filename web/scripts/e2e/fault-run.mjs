// Own-process runner. Never changes shared services or uses their API database.
/* global process, console */
import { createServer } from "vite";
import { mkdtemp, rm, mkdir, writeFile, readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { iceAdapterOptions } from "./browser-options.mjs";
import { safeTarget } from "./harness.mjs";
import { startFaultApi, useFaultRuntime } from "./fault-runtime.mjs";
import { startFaultMedia } from "./media-runtime.mjs";
import { attemptAll } from "./teardown.mjs";

async function recordFailure(kind, record) {
  const report =
    process.env.GELABBER_E2E_REPORT ?? "/tmp/gelabber-e2e/fault-report.json";
  try {
    await mkdir(dirname(report), { recursive: true });
    await writeFile(`${report}.${kind}.json`, JSON.stringify(record) + "\n", {
      mode: 0o600,
    });
  } catch {
    process.exitCode = 1;
    console.error(`FAIL owned-${kind}-report-write-error`);
  }
}

let runtime, mediaRuntime, web, cache, adapterProof;
const digest = async (path) =>
  createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
try {
  const { sfuLoopbackIce } = iceAdapterOptions(process.env);
  const mediaEnv = process.env.GELABBER_E2E_MEDIA_FAULT_ENV;
  const mediaManifest = process.env.GELABBER_E2E_MEDIA_MANIFEST;
  const ownedMediaOrigin = "http://127.0.0.1:18087";
  let media = safeTarget(
    process.env.GELABBER_E2E_MEDIA_ORIGIN ?? ownedMediaOrigin,
  );
  if (mediaEnv || mediaManifest) {
    assert.ok(mediaEnv && mediaManifest, "Both owned media paths required");
    // Validate before acquiring API, proxies or SFU. Attestation must describe
    // the same process to which Vite actually forwards the browser's traffic.
    assert.equal(media, ownedMediaOrigin, "Own SFU media origin mismatch");
  }
  if (sfuLoopbackIce) {
    const root = dirname(process.env.GELABBER_E2E_MEDIA_FAULT_ENV);
    const files = {
      baselineEnv: join(root, "media-fault.env"),
      apiManifest: process.env.GELABBER_E2E_API_MANIFEST,
      mediaManifest: process.env.GELABBER_E2E_MEDIA_MANIFEST,
    };
    adapterProof = {
      name: "explicit-own-SFU-loopback-ICE-topology-control",
      scope: "local Firefox forced-relay only; no default/WAN acceptance",
      nativeMidIndexOverride: false,
      files,
      before: Object.fromEntries(
        await Promise.all(
          Object.entries(files).map(async ([key, path]) => [
            key,
            await digest(path),
          ]),
        ),
      ),
      controlEnvSha256: await digest(process.env.GELABBER_E2E_MEDIA_FAULT_ENV),
    };
  }
  const envPath = process.env.GELABBER_E2E_FAULT_ENV;
  const manifestPath = process.env.GELABBER_E2E_API_MANIFEST;
  const root = process.env.GELABBER_E2E_WEB_SNAPSHOT;
  assert.ok(
    envPath && manifestPath && root,
    "Explicit owned runtime paths required",
  );
  assert.ok(root.startsWith("/"));
  runtime = await startFaultApi({ envPath, manifestPath });
  useFaultRuntime(runtime);
  if (mediaEnv || mediaManifest) {
    mediaRuntime = await startFaultMedia({
      envPath: mediaEnv,
      manifestPath: mediaManifest,
      redisPort: runtime.redis.port,
      loopbackIceAdapter: sfuLoopbackIce,
    });
    assert.equal(
      mediaRuntime.origin,
      ownedMediaOrigin,
      "Own SFU origin mismatch",
    );
    media = mediaRuntime.origin;
    runtime.media = mediaRuntime;
    process.env.GELABBER_E2E_MEDIA_SHA = mediaRuntime.manifest.sourceSha;
  }
  cache = await mkdtemp(join(tmpdir(), "gelabber-e2e-vite-"));
  web = await createServer({
    root,
    configLoader: "runner",
    cacheDir: cache,
    server: {
      host: "127.0.0.1",
      port: 15186,
      strictPort: true,
      proxy: {
        "/api": { target: runtime.origin, changeOrigin: false },
        "/ws": { target: runtime.origin, ws: true, changeOrigin: false },
        "/media": { target: media, ws: true, changeOrigin: false },
      },
    },
    logLevel: "silent",
  });
  await web.listen();
  process.env.GELABBER_SMOKE_URL = "http://127.0.0.1:15186";
  process.env.GELABBER_E2E_API_SHA = runtime.manifest.sourceSha;
  await import("./run.mjs");
} catch {
  // Startup/teardown errors can contain credentials or signed URLs: persist no raw error.
  process.exitCode = 1;
  await recordFailure("startup", {
    status: "BLOCKED",
    reason: "owned-runtime-startup-or-run-error",
    completeAcceptance: false,
    runner: fileURLToPath(import.meta.url),
  });
  console.error(
    "BLOCKED owned-runtime-startup-or-run-error; redacted startup artifact written",
  );
} finally {
  const failedSteps = await attemptAll([
    ["api-resume", () => runtime?.resumeApi?.()],
    ["redis-restore", () => runtime?.redis.restore()],
    ["storage-restore", () => runtime?.storage.restore()],
    ["database-restore", () => runtime?.database.restore()],
    ["web-close", () => web?.close()],
    ["media-runtime-close", () => mediaRuntime?.close()],
    ["runtime-close", () => runtime?.close()],
    ["runtime-clear", () => useFaultRuntime(null)],
    [
      "cache-remove",
      () => cache && rm(cache, { recursive: true, force: true }),
    ],
  ]);
  if (adapterProof) {
    try {
      const after = Object.fromEntries(
        await Promise.all(
          Object.entries(adapterProof.files).map(async ([key, path]) => [
            key,
            await digest(path),
          ]),
        ),
      );
      const baselineUnchanged = Object.keys(after).every(
        (key) => after[key] === adapterProof.before[key],
      );
      if (!baselineUnchanged) process.exitCode = 1;
      await recordFailure("runtime-adapter", {
        status: baselineUnchanged && failedSteps.length === 0 ? "PASS" : "FAIL",
        name: adapterProof.name,
        scope: adapterProof.scope,
        nativeMidIndexOverride: false,
        baselineHashesBefore: adapterProof.before,
        baselineHashesAfter: after,
        baselineUnchanged,
        controlEnvSha256: adapterProof.controlEnvSha256,
        launcherSelectionRestored:
          "baseline; adapter supplied only by explicit per-process selection",
        ownSfu: mediaRuntime?.iceAdapter,
        failedSteps,
        completeAcceptance: false,
      });
    } catch {
      process.exitCode = 1;
      await recordFailure("runtime-adapter", {
        status: "FAIL",
        reason: "adapter-proof-or-restore-error",
        completeAcceptance: false,
      });
    }
  }
  if (failedSteps.length) {
    process.exitCode = 1;
    await recordFailure("teardown", {
      status: "FAIL",
      classification: "test-error",
      reason: "owned-runtime-teardown-failed",
      failedSteps,
      completeAcceptance: false,
    });
    console.error(
      "FAIL owned-runtime-teardown-failed; redacted teardown artifact attempted",
    );
  }
}
