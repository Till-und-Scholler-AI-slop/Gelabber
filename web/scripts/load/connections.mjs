/* global process, console */
// Cost of open gateway connections alone. `prepare` registers the users and
// keeps their session cookies, so a later run measures sockets without the
// password hashing of sign-ins: restart the API between prepare and run.
//   node scripts/load/connections.mjs prepare
//   node scripts/load/connections.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { Client, pool, sleep } from "./client.mjs";
import { phase, resultLog } from "./phases.mjs";

const env = process.env;
const API = env.GELABBER_LOAD_API ?? "http://127.0.0.1:8080";
const STEPS = (env.STEPS ?? "250,500,1000,2000").split(",").map(Number);
const IDLE_S = Number(env.IDLE_S ?? 45);
const USERS_FILE = env.USERS_FILE ?? "connection-users.json";
const TAG = env.TAG ?? "conn";
const log = resultLog(env.RESULTS ?? "connection-results.jsonl");

if (process.argv[2] === "prepare") {
  phase(`${TAG}-prepare`);
  const run = Date.now().toString(36);
  const owner = new Client(API);
  await owner.register(`connowner-${run}@example.test`, "Owner");
  const server = await owner.req("POST", "/servers", {
    name: `Connections ${run}`,
  });
  const channel = server.channels.find((c) => c.kind === "text");
  const invite = await owner.req("POST", `/servers/${server.id}/invites`, {});
  const jars = await pool(
    Array.from({ length: Math.max(...STEPS) }, (_, i) => i),
    24,
    async (i) => {
      const c = new Client(API);
      await c.register(`c${i}-${run}@example.test`, `Conn ${i}`);
      await c.req("POST", `/invites/${invite.code}/join`, {});
      return [...c.jar];
    },
  );
  writeFileSync(
    USERS_FILE,
    JSON.stringify({ server: server.id, channel: channel.id, jars }),
  );
  console.log(
    `${jars.length} users in ${USERS_FILE}; restart the API before the run`,
  );
  process.exit(0);
}

const { server, channel, jars } = JSON.parse(readFileSync(USERS_FILE, "utf8"));
const open = [];
phase(`${TAG}-0`);
await sleep(IDLE_S * 1000);
for (const target of STEPS) {
  phase(`${TAG}-connect-${target}`);
  let failed = 0;
  await pool(jars.slice(open.length, target), 32, async (jar) => {
    const c = new Client(API);
    c.jar = new Map(jar);
    try {
      await c.connect();
      c.subscribe(server, channel);
      open.push(c);
    } catch {
      failed++;
    }
  });
  log({ kind: "connect", target, open: open.length, failed });
  phase(`${TAG}-${target}`);
  await sleep(IDLE_S * 1000);
}
phase(`${TAG}-closed`);
for (const c of open) c.close();
await sleep(IDLE_S * 1000);
phase(`${TAG}-done`);
process.exit(0);
