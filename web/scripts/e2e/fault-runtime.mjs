/* global process, fetch, URL */
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { open, readFile, stat, readdir, readlink } from "node:fs/promises";
import { once } from "node:events";
import { createServer } from "node:net";
import { parseEnv, promisify } from "node:util";
import { setTimeout as pause } from "node:timers/promises";
import { objectStatus } from "./s3-object.mjs";
import { tcpFaultProxy, storageFaultProxy } from "./faults.mjs";
import { deleteTopicKeys, ownedLiveExists } from "./redis-command.mjs";
import {
  closeOwnedApi,
  terminateOwnedChild,
  TeardownFailure,
} from "./teardown.mjs";

const exec = promisify(execFile);
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/;
const local = (u) =>
  assert.ok(
    ["localhost", "127.0.0.1", "[::1]"].includes(u.hostname),
    "Fault runtime must use loopback services",
  );
export async function startFaultApi({
  envPath,
  manifestPath,
  apiPort = 18086,
  redisPort = 16386,
  storagePort = 19086,
}) {
  assert.equal(
    (await stat(envPath)).mode & 0o077,
    0,
    "Private fault env must be mode600",
  );
  const env = parseEnv(await readFile(envPath, "utf8"));
  const db = new URL(env.DATABASE_URL);
  local(db);
  assert.match(
    db.pathname,
    /^\/gelabber_fault_[a-f0-9]+$/,
    "Dedicated fault database required",
  );
  assert.match(
    env.MINIO_BUCKET,
    /^gb-fault-[a-f0-9-]+$/,
    "Dedicated fault bucket required",
  );
  assert.ok(
    env.MINIO_ENDPOINT && env.MINIO_ROOT_USER && env.MINIO_ROOT_PASSWORD,
    "Real MinIO required",
  );
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  assert.match(manifest.sourceSha, /^[a-f0-9]{40}$/);
  assert.match(manifest.sha256, /^[a-f0-9]{64}$/);
  assert.equal(
    createHash("sha256")
      .update(await readFile(manifest.binaryPath))
      .digest("hex"),
    manifest.sha256,
    "API binary provenance mismatch",
  );
  const pgEnv = {
    ...process.env,
    PGHOST: db.hostname,
    PGPORT: db.port || "5432",
    PGUSER: decodeURIComponent(db.username),
    PGPASSWORD: decodeURIComponent(db.password),
    PGDATABASE: db.pathname.slice(1),
  };
  async function sql(statement) {
    // DB and SQL output stay internal. Callers report only bounded counters/statuses.
    return (
      await exec(
        "psql",
        ["-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-c", statement],
        { env: pgEnv },
      )
    ).stdout.trim();
  }
  assert.equal(
    await sql(
      "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()",
    ),
    "0",
    "Another process already uses the fault DB",
  );
  const proxies = [];
  let child,
    logfile,
    paused = false;
  const locks = new Set();
  const topics = new Set();
  async function close() {
    await closeOwnedApi({ locks, proxies, child, logfile });
  }
  try {
    const portProbe = createServer();
    portProbe.listen({ host: "127.0.0.1", port: apiPort, exclusive: true });
    await once(portProbe, "listening");
    await new Promise((r) => portProbe.close(r));
    const redis = await tcpFaultProxy(env.REDIS_URL, redisPort);
    proxies.push(redis);
    const storage = await storageFaultProxy(
      env.MINIO_ENDPOINT,
      env.MINIO_BUCKET,
      storagePort,
    );
    proxies.push(storage);
    const database = await tcpFaultProxy(env.DATABASE_URL, 0);
    proxies.push(database);
    const apiDb = new URL(db);
    apiDb.hostname = "127.0.0.1";
    apiDb.port = String(database.port);
    logfile = await open(
      `/tmp/gelabber-e2e-fault-api-${apiPort}.log`,
      "w",
      0o600,
    );
    child = spawn(manifest.binaryPath, [], {
      env: {
        ...process.env,
        ...env,
        API_ADDR: `127.0.0.1:${apiPort}`,
        DATABASE_URL: apiDb.href,
        REDIS_URL: `redis://127.0.0.1:${redis.port}`,
        MINIO_ENDPOINT: `http://127.0.0.1:${storage.port}`,
        MINIO_PUBLIC_ENDPOINT: `http://127.0.0.1:${storage.port}`,
        API_ALLOW_MEMORY_STORE: "false",
        API_COOKIE_SECURE: "false",
        API_WS_REPLAY: "8",
        API_RATE_AUTH_PER_MIN: "0",
        API_RATE_API_PER_MIN: "0",
        API_RATE_MSG_PER_MIN: "0",
        API_RATE_UPLOAD_PER_HOUR: "0",
      },
      stdio: ["ignore", logfile.fd, logfile.fd],
    });
    let spawnError = false;
    child.on("error", () => {
      spawnError = true;
    });
    const origin = `http://127.0.0.1:${apiPort}`;
    let ready = false;
    const deadline = Date.now() + 30_000;
    do {
      assert.ok(
        !spawnError && child.exitCode === null && child.signalCode === null,
        "Owned API exited during startup",
      );
      ready = await fetch(`${origin}/ready`)
        .then((r) => r.ok)
        .catch(() => false);
      if (ready) break;
      await pause(200);
    } while (Date.now() < deadline);
    assert.ok(ready, "Owned API readiness deadline");
    const redisTarget = new URL(env.REDIS_URL);
    local(redisTarget);
    return {
      origin,
      redis,
      storage,
      database,
      sql,
      close,
      manifest,
      async pauseApi() {
        assert.ok(child.exitCode === null && child.signalCode === null);
        assert.ok(child.kill("SIGSTOP"), "Owned API suspend failed");
        paused = true;
      },
      resumeApi() {
        if (!paused) return;
        assert.ok(child.kill("SIGCONT"), "Owned API resume failed");
        paused = false;
      },
      async liveExists(channel, owner) {
        assert.match(channel, uuid);
        assert.match(owner, uuid);
        assert.equal(
          await sql(
            `SELECT count(*) FROM channels c JOIN servers s ON s.id=c.server_id WHERE c.id='${channel}'::uuid AND s.owner_id='${owner}'::uuid`,
          ),
          "1",
          "Only owned voice fixtures may be inspected",
        );
        return ownedLiveExists(redisTarget.href, channel);
      },
      async lockOwnedServer(server, owner) {
        assert.match(server, uuid);
        assert.match(owner, uuid);
        assert.equal(
          await sql(
            `SELECT count(*) FROM servers WHERE id='${server}'::uuid AND owner_id='${owner}'::uuid`,
          ),
          "1",
          "Only fixture-owned servers may be locked",
        );
        const locker = spawn(
          "psql",
          ["-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1"],
          { env: pgEnv, stdio: ["pipe", "pipe", "ignore"] },
        );
        let ready = false;
        locker.on("error", () => {});
        locker.stdin.on("error", () => {});
        locker.stdout.on("data", (data) => {
          if (String(data).includes("E2E_LOCKED")) ready = true;
        });
        async function release() {
          locks.delete(release);
          if (locker.exitCode !== null || locker.signalCode !== null) return;
          const exited = once(locker, "exit");
          let commitFailed = false;
          try {
            locker.stdin.end("COMMIT;\n\\q\n");
          } catch {
            commitFailed = true;
          }
          await Promise.race([exited, pause(2_000, undefined, { ref: false })]);
          // Failed pipe/commit still releases the owned locker process.
          await terminateOwnedChild(locker, 0);
          if (commitFailed) throw new TeardownFailure(["lock-commit-pipe"]);
        }
        locks.add(release);
        locker.stdin.write(
          `BEGIN;\nSELECT id FROM servers WHERE id='${server}'::uuid FOR UPDATE;\n\\echo E2E_LOCKED\n`,
        );
        const deadline = Date.now() + 5_000;
        while (!ready && locker.exitCode === null && Date.now() < deadline)
          await pause(50);
        if (!ready) {
          await release();
          throw new Error("Owned server lock control failed");
        }
        return {
          release,
          waiting: async () =>
            Number(
              await sql(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%SELECT id FROM servers WHERE id = $1 FOR UPDATE%'",
              ),
            ),
        };
      },
      async processSample(peerPort) {
        const status = await readFile(`/proc/${child.pid}/status`, "utf8");
        const rssKiB = Number(/^VmRSS:\s+(\d+)/m.exec(status)?.[1]);
        assert.ok(Number.isFinite(rssKiB));
        const inodes = new Set();
        for (const fd of await readdir(`/proc/${child.pid}/fd`)) {
          const link = await readlink(`/proc/${child.pid}/fd/${fd}`).catch(
            () => "",
          );
          const socket = /^socket:\[(\d+)\]$/.exec(link);
          if (socket) inodes.add(socket[1]);
        }
        const tcp = await readFile(`/proc/${child.pid}/net/tcp`, "utf8");
        const raw = tcp
          .split("\n")
          .slice(1)
          .map((row) => row.trim().split(/\s+/))
          .find(
            (row) =>
              row.length > 9 &&
              parseInt(row[1].split(":")[1], 16) === apiPort &&
              parseInt(row[2].split(":")[1], 16) === peerPort &&
              inodes.has(row[9]),
          );
        return {
          at: new Date().toISOString(),
          process: "owned-manifest-api-child",
          rssKiB,
          rawSocketOpen: Boolean(raw),
          rawSendQueueBytes: raw ? parseInt(raw[4].split(":")[0], 16) : 0,
        };
      },
      async attachmentObject(id) {
        assert.match(id, uuid);
        const key = await sql(
          `SELECT object_key FROM attachments WHERE id='${id}'::uuid`,
        );
        assert.ok(key.length > 0, "Owned attachment missing");
        return {
          status: () =>
            objectStatus(
              `http://127.0.0.1:${storage.port}`,
              env.MINIO_BUCKET,
              key,
              env.MINIO_ROOT_USER,
              env.MINIO_ROOT_PASSWORD,
            ),
        };
      },
      allowTopic(id) {
        assert.match(id, uuid);
        topics.add(id);
      },
      async resetTopic(id, kind = "c") {
        assert.ok(topics.has(id), "Refusing a foreign Redis topic");
        assert.ok(["c", "s"].includes(kind));
        const key = `gb:n:${kind}:${id}`;
        return deleteTopicKeys(redisTarget.href, [
          key,
          `${key}:ep`,
          `gb:l:${kind}:${id}`,
          `${key}:delivery`,
        ]);
      },
    };
  } catch (error) {
    await close();
    throw error;
  }
}

export let activeFaultRuntime = null;
export function useFaultRuntime(runtime) {
  activeFaultRuntime = runtime;
}
