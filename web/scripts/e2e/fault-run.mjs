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
  const report =
    process.env.GELABBER_E2E_REPORT ?? "/tmp/gelabber-e2e/fault-startup.json";
  await mkdir(dirname(report), { recursive: true });
  await writeFile(
    `${report}.startup.json`,
    JSON.stringify({
      status: "BLOCKED",
      reason: "owned-runtime-startup-or-run-error",
      completeAcceptance: false,
      runner: fileURLToPath(import.meta.url),
    }) + "\n",
    { mode: 0o600 },
  );
  console.error(
    "BLOCKED owned-runtime-startup-or-run-error; redacted startup artifact written",
  );
} finally {
  runtime?.redis.restore();
  runtime?.storage.restore();
  runtime?.database.restore();
  await web?.close();
  await runtime?.close();
  useFaultRuntime(null);
  if (cache) await rm(cache, { recursive: true, force: true });
}
