/* global Buffer, process, setTimeout, clearTimeout */
// One `node` child for scripts/checks.mjs: its output is held until it ends,
// and it is killed when its time runs out, so a hung check names itself
// instead of running into the CI job's own limit. The child runs in a process
// group of its own, and the whole group is killed: the browsers a check
// started go with it instead of loading the next checks.
import { spawn } from "node:child_process";

const groups = new Set();
function killGroup(pid) {
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    // Already gone.
  }
}
let trapped = false;
// A group of its own no longer gets the terminal's Ctrl-C: pass it on.
function trapSignals() {
  if (trapped) return;
  trapped = true;
  for (const signal of ["SIGINT", "SIGTERM"])
    process.on(signal, () => {
      for (const pid of groups) killGroup(pid);
      process.exit(130);
    });
}

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
      detached: process.platform !== "win32",
    });
    trapSignals();
    if (child.pid) groups.add(child.pid);
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
      if (process.platform === "win32") child.kill("SIGKILL");
      else killGroup(child.pid);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      groups.delete(child.pid);
      resolve({ code, killed, output: Buffer.concat(output) });
    });
  });
}
