/* global Buffer, process, setTimeout, clearTimeout */
// One `node` child for scripts/checks.mjs: its output is held until it ends,
// and it is killed when its time runs out, so a hung check names itself
// instead of running into the CI job's own limit.
import { spawn } from "node:child_process";

/**
 * Runs `node ...args`. Resolves with its exit code, whether it was killed,
 * and everything it wrote. `expired` resolves when its time is up; it gets
 * `started`, which resolves on the child's first output (for tests). The
 * default is `limitMs` after the start.
 */
export function runWithin(args, { limitMs = 240_000, expired, env } = {}) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, args, {
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, ...env },
    });
    const output = [];
    let wrote;
    const started = new Promise((resolve) => {
      wrote = resolve;
    });
    const keep = (chunk) => {
      output.push(chunk);
      wrote();
    };
    child.stdout.on("data", keep);
    child.stderr.on("data", keep);
    let timer;
    let killed = false;
    const due = expired
      ? expired({ started })
      : new Promise((resolve) => {
          timer = setTimeout(resolve, limitMs);
        });
    due.then(() => {
      if (child.exitCode !== null || child.signalCode !== null) return;
      killed = true;
      child.kill("SIGKILL");
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code, killed, output: Buffer.concat(output) });
    });
  });
}
