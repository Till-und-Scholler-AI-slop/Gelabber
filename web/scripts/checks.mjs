/* global console, process */
// The E2E harness and script checks the Frontend CI job runs. Every check
// runs; the exit code is nonzero if any failed.
import { spawnSync } from "node:child_process";

const vm = "--experimental-vm-modules";
const checks = [
  ["scripts/e2e/harness.checks.mjs"],
  ["scripts/e2e/rendered-frames.checks.mjs"],
  ["scripts/e2e/media-probe.checks.mjs"],
  ["scripts/e2e/publisher-encoders.checks.mjs"],
  ["scripts/e2e/receive-loss-proxy.checks.mjs"],
  ["scripts/e2e/browser-options.checks.mjs"],
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
];

const failed = checks.filter(
  (args) =>
    spawnSync(process.execPath, args, { stdio: "inherit" }).status !== 0,
);
for (const args of failed) console.error(`Failed: node ${args.join(" ")}`);
process.exit(failed.length ? 1 : 0);
