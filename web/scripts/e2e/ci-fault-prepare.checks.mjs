/* global URL */
// Execute the preparer's actual source with only CI infrastructure substituted.
import { test } from "node:test";
import { Buffer } from "node:buffer";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { parseEnv, promisify } from "node:util";

const secret = "PRIVATE-connection-secret";
async function prepareProbe(overrides = {}, failSql = false) {
  const calls = [],
    files = new Map(),
    messages = [];
  const binary = Buffer.from("owned-CI-test-binary");
  const fakeProcess = {
    exitCode: 0,
    env: {
      CI: "true",
      DATABASE_URL: `postgres://fixture:${secret}@127.0.0.1:5432/control`,
      REDIS_URL: "redis://127.0.0.1:6379",
      MINIO_ENDPOINT: "http://127.0.0.1:9000",
      MINIO_ROOT_USER: "fixture",
      MINIO_ROOT_PASSWORD: secret,
      GITHUB_ENV: "/tmp/fixture-github-env",
      ...overrides,
    },
  };
  const execFile = () => {};
  execFile[promisify.custom] = async (command, args, options) => {
    calls.push({ command, args, options });
    if (command === "git") return { stdout: "a".repeat(40) + "\n" };
    assert.equal(command, "psql");
    if (failSql) throw new Error(secret);
    return { stdout: "CREATE DATABASE\n" };
  };
  const mocks = {
    "node:child_process": { execFile },
    "node:fs/promises": {
      async readFile(path) {
        return files.get(path)?.body ?? binary;
      },
      async mkdtemp() {
        return "/tmp/fixture-owned-ci";
      },
      async copyFile(_from, to) {
        files.set(to, { body: binary });
      },
      async writeFile(path, body, options) {
        files.set(path, { body, options });
      },
      async appendFile(path, body) {
        files.set(path, { body });
      },
    },
  };
  const context = vm.createContext({
    process: fakeProcess,
    URL,
    console: { error: (message) => messages.push(message) },
  });
  const source = await readFile(
    new URL("./ci-fault-prepare.mjs", import.meta.url),
    "utf8",
  );
  const module = new vm.SourceTextModule(source, { context });
  await module.link(async (name) => {
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
  await module.evaluate();
  return { calls, files, messages, exitCode: fakeProcess.exitCode };
}

test("CI preparer refuses non-CI, foreign services and missing required variables before creating a DB", async () => {
  for (const overrides of [
    { CI: "false" },
    { DATABASE_URL: "postgres://fixture@foreign.invalid/control" },
    { REDIS_URL: "redis://foreign.invalid:6379" },
    { MINIO_ENDPOINT: "http://foreign.invalid:9000" },
    { MINIO_ROOT_PASSWORD: "" },
    { GITHUB_ENV: undefined },
  ]) {
    const result = await prepareProbe(overrides);
    assert.equal(result.exitCode, 1);
    assert.equal(result.calls.length, 0);
    assert.equal(result.files.size, 0);
    assert.ok(!JSON.stringify(result.messages).includes(secret));
  }
});
test("CI preparer creates a distinct DB/bucket, private env and byte-verified API manifest without credentials in argv/output", async () => {
  const result = await prepareProbe();
  assert.equal(result.exitCode, 0);
  assert.deepEqual(result.messages, []);
  const sql = result.calls.find((call) => call.command === "psql");
  assert.match(
    sql.args.at(-1),
    /^CREATE DATABASE gelabber_fault_[a-f0-9]{24}$/,
  );
  assert.equal(sql.options.env.PGDATABASE, "control");
  assert.equal(sql.options.env.PGPASSWORD, secret);
  assert.ok(
    !JSON.stringify(result.calls.map(({ args }) => args)).includes(secret),
  );
  const config = result.files.get("/tmp/fixture-owned-ci/fault.env");
  assert.equal(config.options.mode, 0o600);
  const env = parseEnv(config.body);
  const db = new URL(env.DATABASE_URL);
  assert.equal(db.pathname.slice(1), sql.args.at(-1).split(" ").at(-1));
  assert.equal(
    env.MINIO_BUCKET,
    db.pathname.replace("/gelabber_fault_", "gb-fault-"),
  );
  assert.equal(env.MINIO_ROOT_PASSWORD, secret);
  const manifestFile = result.files.get(
    "/tmp/fixture-owned-ci/api-manifest.json",
  );
  assert.equal(manifestFile.options.mode, 0o600);
  const manifest = JSON.parse(manifestFile.body);
  assert.equal(manifest.sourceSha, "a".repeat(40));
  assert.equal(
    manifest.sha256,
    createHash("sha256")
      .update(result.files.get(manifest.binaryPath).body)
      .digest("hex"),
  );
  const exported = result.files.get("/tmp/fixture-github-env").body;
  assert.ok(exported.includes("GELABBER_E2E_FAULT_ENV="));
  assert.ok(exported.includes("GELABBER_E2E_WEB_SHA=" + "a".repeat(40)));
  assert.ok(!exported.includes(secret));
});
test("failed CI SQL setup remains nonzero and redacted, without exporting an unusable runtime", async () => {
  const result = await prepareProbe({}, true);
  assert.equal(result.exitCode, 1);
  assert.equal(result.files.size, 0);
  assert.ok(!JSON.stringify(result.messages).includes(secret));
  assert.deepEqual(result.messages, [
    "FAIL owned-CI-runtime-preparation; private details redacted",
  ]);
});
