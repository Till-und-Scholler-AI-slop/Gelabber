// Own-process runner. Never changes shared services or uses their API database.
/* global process, console */
import { createServer } from "vite";
import { mkdtemp, rm, mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { safeTarget } from "./harness.mjs";
import { startFaultApi, useFaultRuntime } from "./fault-runtime.mjs";
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
    console.error(`FAIL owned-${kind}-report-write-error`);
  }
}

let runtime, web, cache;
try {
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
  const media = safeTarget(
    process.env.GELABBER_E2E_MEDIA_ORIGIN ?? "http://127.0.0.1:18087",
  );
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
    ["redis-restore", () => runtime?.redis.restore()],
    ["storage-restore", () => runtime?.storage.restore()],
    ["database-restore", () => runtime?.database.restore()],
    ["web-close", () => web?.close()],
    ["runtime-close", () => runtime?.close()],
    ["runtime-clear", () => useFaultRuntime(null)],
    [
      "cache-remove",
      () => cache && rm(cache, { recursive: true, force: true }),
    ],
  ]);
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
