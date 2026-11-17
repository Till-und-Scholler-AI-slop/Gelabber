/* global console */
// Pure parts of the load test client (no server): the cookie jar, the CSRF
// token, the bounded pool and the metrics parser.
// Run in web/: node scripts/load/load.checks.mjs
import assert from "node:assert/strict";
import {
  Client,
  checkTarget,
  isLoopback,
  parseMetrics,
  pool,
  sleep,
} from "./client.mjs";

const c = new Client("http://127.0.0.1:8080");
c.absorb([
  "gelabber_session=abc; Path=/; HttpOnly; SameSite=Lax",
  "gelabber_csrf=tok=en; Path=/; SameSite=Lax",
]);
assert.equal(c.cookieHeader(), "gelabber_session=abc; gelabber_csrf=tok=en");
assert.equal(c.csrf(), "tok=en");
c.absorb(["gelabber_session=def; Path=/"]);
assert.equal(c.jar.get("gelabber_session"), "def");

let running = 0;
let peak = 0;
const out = await pool([5, 1, 3, 2, 4], 2, async (ms, i) => {
  running++;
  peak = Math.max(peak, running);
  await sleep(ms);
  running--;
  return i * 10;
});
assert.deepEqual(out, [0, 10, 20, 30, 40]);
assert.equal(peak, 2);
assert.deepEqual(await pool([], 4, async () => 1), []);

const m = parseMetrics(
  [
    "# HELP gelabber_mediasoup_consumers Consumers",
    "gelabber_mediasoup_consumers 182",
    'gelabber_media_worker_peers{worker="0"} 14',
    "gelabber_sfu_forwarded_bytes_total 2.5e+06",
  ].join("\n"),
);
assert.deepEqual(m, {
  mediasoup_consumers: 182,
  sfu_forwarded_bytes_total: 2500000,
});

// Accounts with a public password only on this machine, unless allowed.
for (const url of [
  "http://127.0.0.1:8080",
  "http://127.1.2.3:4173",
  "http://localhost:8080",
  "http://[::1]:8080",
])
  assert.ok(isLoopback(url), url);
for (const url of [
  "https://gelabber.com",
  "http://10.0.0.2:8080",
  "http://127.0.0.1.example.com",
])
  assert.ok(!isLoopback(url), url);
assert.throws(() => checkTarget("https://gelabber.com", {}), /ALLOW_REMOTE/);
assert.throws(() => new Client("https://gelabber.com"), /ALLOW_REMOTE/);
checkTarget("https://gelabber.com", { GELABBER_LOAD_ALLOW_REMOTE: "1" });

console.log("load checks passed");
