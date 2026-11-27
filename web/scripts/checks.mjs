/* global Buffer, console, process */
// The script checks the Frontend CI job runs: the E2E harness checks, and the
// browser smokes that need only Playwright's Chromium (they build the app and
// answer /api and the gateway themselves). Every check runs; the exit code is
// nonzero if any failed. The smokes run side by side, on half the cores, with
// each one's output printed once it ends.
import { spawn, spawnSync } from "node:child_process";
import { availableParallelism } from "node:os";

const vm = "--experimental-vm-modules";
const checks = [
  ["scripts/e2e/harness.checks.mjs"],
  ["scripts/e2e/rendered-frames.checks.mjs"],
  ["scripts/e2e/media-probe.checks.mjs"],
  ["scripts/e2e/publisher-encoders.checks.mjs"],
  ["scripts/e2e/native-evaluate.checks.mjs"],
  [vm, "scripts/e2e/access-native.checks.mjs"],
  ["scripts/e2e/held-renderer.checks.mjs"],
  [vm, "scripts/e2e/native-finish.checks.mjs"],
  [vm, "scripts/e2e/media-extra.checks.mjs"],
  ["scripts/smoke-netem.checks.mjs"],
  ["scripts/e2e/faults.checks.mjs"],
  [vm, "scripts/e2e/teardown.checks.mjs"],
  [vm, "scripts/e2e/media-runtime.checks.mjs"],
  [vm, "scripts/e2e/ci-fault-prepare.checks.mjs"],
  ["scripts/e2e/ci-fault-loop.checks.mjs"],
  ["scripts/load/load.checks.mjs"],
];
// Longest first, so the slowest one does not start last.
const smokes = [
  ["scripts/smoke-chat-scroll.mjs"],
  ["scripts/smoke-phone-layout.mjs"],
  ["scripts/smoke-lazy-parts.mjs"],
  ["scripts/smoke-member-list.mjs"],
];

// The harness checks first and alone: some of them bound real timers.
const failed = checks.filter(
  (args) =>
    spawnSync(process.execPath, args, { stdio: "inherit" }).status !== 0,
);

const run = (args) =>
  new Promise((resolve) => {
    const child = spawn(process.execPath, args, {
      stdio: ["ignore", "pipe", "pipe"],
    });
    const output = [];
    child.stdout.on("data", (chunk) => output.push(chunk));
    child.stderr.on("data", (chunk) => output.push(chunk));
    child.on("close", (code) => {
      console.log(`--- node ${args.join(" ")}`);
      process.stdout.write(Buffer.concat(output));
      if (code !== 0) failed.push(args);
      resolve();
    });
  });
const queue = [...smokes];
const lanes = Math.max(1, Math.floor(availableParallelism() / 2));
await Promise.all(
  Array.from({ length: Math.min(lanes, queue.length) }, async () => {
    while (queue.length) await run(queue.shift());
  }),
);

for (const args of failed) console.error(`Failed: node ${args.join(" ")}`);
process.exit(failed.length ? 1 : 0);
