/* global process, console, fetch */
// Voice or video call load with the real web client: an observer browser
// joins first, then bot participants join in STEPS. Bots share browsers
// (PER_BROWSER contexts each); the observer has its own, so sampler.py can
// measure it alone. The container's CPU usually ends this test before the
// server's memory does: every participant encodes and decodes.
import { Client, parseMetrics, pool, sleep } from "./client.mjs";
import { launch, signedInPage } from "./browser.mjs";
import { phase, resultLog } from "./phases.mjs";

const env = process.env;
const WEB = env.GELABBER_LOAD_WEB ?? "http://127.0.0.1:4173";
const MEDIA_METRICS =
  env.GELABBER_LOAD_MEDIA_METRICS ?? "http://127.0.0.1:8081/metrics";
const MODE = env.MODE ?? "audio"; // audio | video
const STEPS = (env.STEPS ?? "4,6,8,12").split(",").map(Number);
const HOLD_S = Number(env.HOLD_S ?? 40);
const SETTLE_S = Number(env.SETTLE_S ?? 15);
const PER_BROWSER = Number(env.PER_BROWSER ?? 6);
const RUN = env.RUN ?? Date.now().toString(36);
const log = resultLog(env.RESULTS ?? "call-results.jsonl");
const metrics = async () =>
  parseMetrics(await (await fetch(MEDIA_METRICS)).text());

phase(`call-${MODE}-setup`);
const owner = new Client(WEB);
await owner.register(`callowner-${RUN}@example.test`, "Owner");
const server = await owner.req("POST", "/servers", {
  name: `Call load ${RUN}`,
});
const voice = server.channels.find((c) => c.kind === "voice");
const invite = await owner.req("POST", `/servers/${server.id}/invites`, {});
const path = `/s/${server.id}/c/${voice.id}`;

async function member(name) {
  const c = new Client(WEB);
  await c.register(`${name.toLowerCase()}-${RUN}@example.test`, name);
  await c.req("POST", `/invites/${invite.code}/join`, {});
  return c;
}
async function join(page) {
  await page
    .getByRole("button", { name: "Beitreten", exact: true })
    .first()
    .click({ timeout: 30_000 });
  await page
    .getByRole("button", { name: "Verlassen", exact: true })
    .waitFor({ timeout: 30_000 });
  if (MODE === "video") {
    await sleep(1500);
    await page
      .getByRole("button", { name: "Kamera", exact: true })
      .click({ timeout: 15_000 });
  }
}

const observer = await launch(`web-call-${MODE}`, [
  "--enable-precise-memory-info",
]);
const { page: observerPage } = await signedInPage(
  observer.browser,
  await member("Beobachter"),
  path,
);
phase(`call-${MODE}-observer-idle`);
await sleep(30_000);
await join(observerPage);

const browsers = [];
const bots = [];
let participants = 1;
let nextBot = 0;
for (const target of STEPS) {
  phase(`call-${MODE}-joining-${target}`);
  const ids = Array.from(
    { length: Math.max(0, target - participants) },
    () => nextBot++,
  );
  for (const i of ids) {
    const b = Math.floor(i / PER_BROWSER);
    browsers[b] ??= await launch(`bots-${MODE}-${b}`);
  }
  let failed = 0;
  await pool(ids, 3, async (i) => {
    try {
      const b = browsers[Math.floor(i / PER_BROWSER)];
      const { context, page } = await signedInPage(
        b.browser,
        await member(`Bot${i}`),
        path,
        { width: 960, height: 600 },
      );
      await page.waitForTimeout(2000);
      await join(page);
      bots.push(context);
    } catch (error) {
      failed++;
      console.log(`bot ${i} failed: ${String(error.message).split("\n")[0]}`);
    }
  });
  participants += ids.length - failed;
  await sleep(SETTLE_S * 1000);
  const before = await metrics();
  phase(`call-${MODE}-${participants}`);
  await sleep(HOLD_S * 1000);
  const after = await metrics();
  const record = {
    kind: "step",
    mode: MODE,
    participants,
    failed,
    peers: after.media_peers,
    producers: after.mediasoup_producers,
    consumers: after.mediasoup_consumers,
    forwardedMbitPerSecond:
      ((after.sfu_forwarded_bytes_total - before.sfu_forwarded_bytes_total) *
        8) /
      HOLD_S /
      1e6,
  };
  log(record);
  console.log(record);
}

phase(`call-${MODE}-leaving`);
for (const context of bots) await context.close().catch(() => {});
await observerPage
  .getByRole("button", { name: "Verlassen", exact: true })
  .click()
  .catch(() => {});
phase(`call-${MODE}-after`);
await sleep(40_000);
log({ kind: "after", mode: MODE, ...(await metrics()) });
for (const b of browsers) await b.close();
await observer.close();
phase(`call-${MODE}-done`);
process.exit(0);
