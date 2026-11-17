/* global process, console */
// Text chat load: STEPS of connected users on one server, all subscribed to
// one channel; per step an idle phase, a message burst and a drain phase.
// See README.md.
import { Client, pool, sleep } from "./client.mjs";
import { phase, resultLog } from "./phases.mjs";

const env = process.env;
const API = env.GELABBER_LOAD_API ?? "http://127.0.0.1:8080";
const WEB = env.GELABBER_LOAD_WEB ?? "http://127.0.0.1:4173";
const STEPS = (env.STEPS ?? "100,250,500,1000").split(",").map(Number);
const RATE = Number(env.RATE ?? 20);
const BURST_S = Number(env.BURST_S ?? 60);
const IDLE_S = Number(env.IDLE_S ?? 30);
const ON_TIME_MS = Number(env.ON_TIME_MS ?? 3000);
const RUN = env.RUN ?? Date.now().toString(36);
const log = resultLog(env.RESULTS ?? "chat-results.jsonl");

phase("chat-setup");
const owner = new Client(API);
await owner.register(`owner-${RUN}@example.test`, "Owner");
const server = await owner.req("POST", "/servers", {
  name: `Chat load ${RUN}`,
});
const channel = server.channels.find((c) => c.kind === "text");
const invite = await owner.req("POST", `/servers/${server.id}/invites`, {});

// Optional observer: the real web client, measured as its own process tree.
let observer = null;
if (env.OBSERVER) {
  const { launch, signedInPage } = await import("./browser.mjs");
  const client = new Client(WEB);
  await client.register(`observer-${RUN}@example.test`, "Beobachter");
  await client.req("POST", `/invites/${invite.code}/join`, {});
  const browser = await launch("web-chat", ["--enable-precise-memory-info"]);
  const { page } = await signedInPage(
    browser.browser,
    client,
    `/s/${server.id}/c/${channel.id}`,
  );
  observer = { browser, page };
  phase("chat-observer-idle");
  await sleep(IDLE_S * 1000);
}
async function observerSample() {
  if (!observer) return null;
  return observer.page
    .evaluate(() => ({
      heapBytes: globalThis.performance.memory.usedJSHeapSize,
      domNodes: globalThis.document.getElementsByTagName("*").length,
    }))
    .catch(() => null);
}

const users = [];
const sentAt = new Map(); // message nonce -> send time
let received = 0;
let onTime = 0;
let delaySum = 0;
function onFrame(frame) {
  if (frame.op !== "e") return;
  const m = /load-(\d+)/.exec(JSON.stringify(frame));
  if (!m || !sentAt.has(m[1])) return;
  const delay = Date.now() - sentAt.get(m[1]);
  received++;
  delaySum += delay;
  if (delay <= ON_TIME_MS) onTime++;
}

let seq = 0;
for (const target of STEPS) {
  phase(`chat-connect-${target}`);
  const started = Date.now();
  const fresh = Array.from(
    { length: target - users.length },
    (_, i) => users.length + i,
  );
  await pool(fresh, 24, async (i) => {
    const c = new Client(API);
    await c.register(`u${i}-${RUN}@example.test`, `User ${i}`);
    await c.req("POST", `/invites/${invite.code}/join`, {});
    await c.connect(onFrame);
    c.subscribe(server.id, channel.id);
    users.push(c);
  });
  log({
    kind: "connect",
    users: users.length,
    seconds: (Date.now() - started) / 1000,
  });

  phase(`chat-idle-${target}`);
  await sleep(IDLE_S * 1000);

  phase(`chat-burst-${target}`);
  received = onTime = delaySum = 0;
  sentAt.clear();
  let sent = 0;
  let failed = 0;
  const burstStart = Date.now();
  const inflight = [];
  while (Date.now() - burstStart < BURST_S * 1000) {
    const nonce = String(seq++);
    const author = users[Math.floor(Math.random() * users.length)];
    sentAt.set(nonce, Date.now());
    inflight.push(
      author
        .req("POST", `/channels/${channel.id}/messages`, {
          content: `Lasttest load-${nonce} ${"lorem ipsum ".repeat(8)}`,
        })
        .then(
          () => sent++,
          () => failed++,
        ),
    );
    if (inflight.length >= 200) await Promise.all(inflight.splice(0));
    await sleep(1000 / RATE);
  }
  await Promise.all(inflight);
  const burstSeconds = (Date.now() - burstStart) / 1000;
  const expected = sent * users.length;

  phase(`chat-drain-${target}`);
  const drainStart = Date.now();
  while (received < expected && Date.now() - drainStart < IDLE_S * 1000)
    await sleep(250);
  const record = {
    kind: "burst",
    users: users.length,
    rate: RATE,
    sent,
    failed,
    expected,
    received,
    onTime,
    onTimeMs: ON_TIME_MS,
    deliveredPerSecond: Math.round(
      received / (burstSeconds + (Date.now() - drainStart) / 1000),
    ),
    averageDelayMs: received ? Math.round(delaySum / received) : null,
    observer: await observerSample(),
  };
  log(record);
  console.log(record);
}

phase("chat-disconnect");
for (const u of users) u.close();
await sleep(IDLE_S * 1000);
phase("chat-done");
if (observer) await observer.browser.close();
process.exit(0);
