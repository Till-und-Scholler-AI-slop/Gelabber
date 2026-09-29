/* global process, URL, console */
// CI-only fixture preparation. No credentials or signed URLs are printed.
import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  appendFile,
  copyFile,
  mkdtemp,
  readFile,
  writeFile,
} from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir, networkInterfaces } from "node:os";
import { isIP } from "node:net";

async function prepare() {
  assert.equal(
    process.env.CI,
    "true",
    "This preparer is only for an ephemeral CI worker",
  );
  const exec = promisify(execFile);
  const db = new URL(process.env.DATABASE_URL);
  assert.equal(db.protocol, "postgres:");
  assert.ok(process.env.GITHUB_ENV?.startsWith("/"));
  assert.ok(["localhost", "127.0.0.1"].includes(db.hostname));
  assert.ok(
    process.env.MINIO_ENDPOINT &&
      process.env.MINIO_ROOT_USER &&
      process.env.MINIO_ROOT_PASSWORD,
  );
  for (const [value, protocols] of [
    [process.env.REDIS_URL, ["redis:"]],
    [process.env.MINIO_ENDPOINT, ["http:"]],
  ]) {
    const service = new URL(value);
    assert.ok(["localhost", "127.0.0.1"].includes(service.hostname));
    assert.ok(protocols.includes(service.protocol));
  }
  const sourceSha = (await exec("git", ["rev-parse", "HEAD"])).stdout.trim();
  assert.match(sourceSha, /^[a-f0-9]{40}$/);
  const binary = await readFile(resolve("target/debug/gelabber-api"));
  const sha256 = createHash("sha256").update(binary).digest("hex");
  const withMedia = process.env.GELABBER_E2E_PREPARE_MEDIA === "true";
  let mediaBinary, iceAddress;
  if (withMedia) {
    mediaBinary = await readFile(resolve("target/debug/gelabber-media"));
    iceAddress = Object.values(networkInterfaces())
      .flat()
      .find((item) => item?.family === "IPv4" && !item.internal)?.address;
    assert.equal(
      isIP(iceAddress ?? ""),
      4,
      "A real local IPv4 is required for native Firefox ICE",
    );
  }
  const suffix = randomBytes(12).toString("hex");
  const name = `gelabber_fault_${suffix}`;
  await exec(
    "psql",
    ["-X", "-v", "ON_ERROR_STOP=1", "-c", `CREATE DATABASE ${name}`],
    {
      env: {
        ...process.env,
        PGHOST: db.hostname,
        PGPORT: db.port || "5432",
        PGUSER: decodeURIComponent(db.username),
        PGPASSWORD: decodeURIComponent(db.password),
        PGDATABASE: db.pathname.slice(1),
      },
    },
  );
  db.pathname = `/${name}`;
  const dir = await mkdtemp(join(tmpdir(), "gelabber-e2e-ci-runtime-"));
  const binaryPath = join(dir, "api");
  await copyFile(resolve("target/debug/gelabber-api"), binaryPath);
  assert.equal(
    createHash("sha256")
      .update(await readFile(binaryPath))
      .digest("hex"),
    sha256,
  );
  const manifestPath = join(dir, "api-manifest.json");
  await writeFile(
    manifestPath,
    JSON.stringify({
      sourceSha,
      binaryPath,
      sha256,
      build: "cargo build --locked -p gelabber-api; current CI checkout",
    }) + "\n",
    { mode: 0o600 },
  );
  const envPath = join(dir, "fault.env");
  const env = {
    DATABASE_URL: db.href,
    REDIS_URL: process.env.REDIS_URL,
    MINIO_ENDPOINT: process.env.MINIO_ENDPOINT,
    MINIO_ROOT_USER: process.env.MINIO_ROOT_USER,
    MINIO_ROOT_PASSWORD: process.env.MINIO_ROOT_PASSWORD,
    MINIO_BUCKET: `gb-fault-${suffix}`,
  };
  await writeFile(
    envPath,
    Object.entries(env)
      .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
      .join("\n") + "\n",
    { mode: 0o600 },
  );
  const mediaExports = [];
  if (withMedia) {
    const mediaPath = join(dir, "media");
    await copyFile(resolve("target/debug/gelabber-media"), mediaPath);
    const mediaSha = createHash("sha256").update(mediaBinary).digest("hex");
    assert.equal(
      createHash("sha256")
        .update(await readFile(mediaPath))
        .digest("hex"),
      mediaSha,
    );
    const mediaManifest = join(dir, "media-manifest.json");
    await writeFile(
      mediaManifest,
      JSON.stringify({
        sourceSha,
        binaryPath: mediaPath,
        sha256: mediaSha,
        build: "cargo build --locked -p gelabber-media; current CI checkout",
      }) + "\n",
      { mode: 0o600 },
    );
    const mediaEnv = join(dir, "media-fault.env");
    const configuration = {
      MEDIA_ADDR: "127.0.0.1:18087",
      REDIS_URL: process.env.REDIS_URL,
      MEDIA_ICE_BIND: `${iceAddress}:0`,
      MEDIA_ADVERTISED_IP: iceAddress,
    };
    await writeFile(
      mediaEnv,
      Object.entries(configuration)
        .map(([key, value]) => `${key}=${JSON.stringify(value)}`)
        .join("\n") + "\n",
      { mode: 0o600 },
    );
    mediaExports.push(
      `GELABBER_E2E_MEDIA_FAULT_ENV=${mediaEnv}`,
      `GELABBER_E2E_MEDIA_MANIFEST=${mediaManifest}`,
    );
  }
  await appendFile(
    process.env.GITHUB_ENV,
    [
      `GELABBER_E2E_FAULT_ENV=${envPath}`,
      `GELABBER_E2E_API_MANIFEST=${manifestPath}`,
      `GELABBER_E2E_WEB_SNAPSHOT=${resolve("web")}`,
      `GELABBER_E2E_WEB_SHA=${sourceSha}`,
      ...mediaExports,
    ].join("\n") + "\n",
  );
}

try {
  await prepare();
} catch {
  // Subprocess failures can contain private connection details. Never emit them.
  process.exitCode = 1;
  console.error("FAIL owned-CI-runtime-preparation; private details redacted");
}
