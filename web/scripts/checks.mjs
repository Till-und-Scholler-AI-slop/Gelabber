/* global console, process */
// The script checks the Frontend CI job runs: the E2E harness checks, and the
// browser smokes that need only Playwright's Chromium (they build the app and
// answer /api and the gateway themselves). Every check runs; the exit code is
// nonzero if any failed. The smokes run side by side, on half the cores. Each
// check's output is printed once it ends, and one that runs past its time is
// killed and counted as failed.
import { availableParallelism } from "node:os";
import { runWithin } from "./run-within.mjs";

const LIMIT_S = 240;

const vm = "--experimental-vm-modules";
const checks = [
  ["scripts/e2e/harness.checks.mjs"],
  ["scripts/e2e/rendered-frames.checks.mjs"],
  ["scripts/e2e/media-probe.checks.mjs"],
  ["scripts/e2e/publisher-encoders.checks.mjs"],
  ["scripts/e2e/browser-options.checks.mjs"],
  ["scripts/e2e/native-evaluate.checks.mjs"],
  [vm, "scripts/e2e/access-native.checks.mjs"],
  ["scripts/e2e/held-renderer.checks.mjs"],
  [vm, "scripts/e2e/native-finish.checks.mjs"],
  [vm, "scripts/e2e/media-extra.checks.mjs"],
  ["scripts/smoke-netem.checks.mjs"],
  ["scripts/prerequisite.checks.mjs"],
  ["scripts/e2e/faults.checks.mjs"],
  [vm, "scripts/e2e/teardown.checks.mjs"],
  [vm, "scripts/e2e/media-runtime.checks.mjs"],
  [vm, "scripts/e2e/ci-fault-prepare.checks.mjs"],
  ["scripts/e2e/ci-fault-loop.checks.mjs"],
  ["scripts/e2e/extended-issue.checks.mjs"],
  ["scripts/e2e/expected-failures.checks.mjs"],
  ["scripts/load/load.checks.mjs"],
  ["scripts/audio/benchmark-rnnoise.mjs"],
  ["scripts/run-within.checks.mjs"],
];
// Longest first, so the slowest one does not start last.
const smokes = [
  ["scripts/smoke-chat-scroll.mjs"],
  ["scripts/smoke-phone-layout.mjs"],
  ["scripts/smoke-lazy-parts.mjs"],
  ["scripts/smoke-member-list.mjs"],
];

const failed = [];
async function run(args) {
  const result = await runWithin(args, { limitMs: LIMIT_S * 1000 });
  console.log(
    `--- node ${args.join(" ")}${result.killed ? `: killed after ${LIMIT_S} s` : ""}`,
  );
  process.stdout.write(result.output);
  if (result.killed || result.code !== 0) failed.push(args);
}

// The harness checks first and one at a time: some of them bound real timers.
for (const args of checks) await run(args);

const queue = [...smokes];
const lanes = Math.max(1, Math.floor(availableParallelism() / 2));
await Promise.all(
  Array.from({ length: Math.min(lanes, queue.length) }, async () => {
    while (queue.length) await run(queue.shift());
  }),
);

for (const args of failed) console.error(`Failed: node ${args.join(" ")}`);
process.exit(failed.length ? 1 : 0);
