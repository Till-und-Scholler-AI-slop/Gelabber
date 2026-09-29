import { setTimeout as pause } from "node:timers/promises";

export class TeardownFailure extends Error {
  constructor(steps) {
    super("owned-teardown-failed");
    this.name = "TeardownFailure";
    this.steps = steps;
  }
}

// Failures carry only controlled step names, never underlying exception text.
export async function attemptAll(actions) {
  const failures = [];
  for (const [step, action] of actions) {
    try {
      await action();
    } catch (error) {
      failures.push(step);
      if (error instanceof TeardownFailure) failures.push(...error.steps);
    }
  }
  return [...new Set(failures)];
}

export async function terminateOwnedChild(child, graceMs = 10_000) {
  const running = () =>
    child?.pid && child.exitCode === null && child.signalCode === null;
  if (!running()) return;
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const failures = await attemptAll([
    ["child-sigterm", () => child.kill("SIGTERM")],
    [
      "child-grace-wait",
      () => Promise.race([exited, pause(graceMs, undefined, { ref: false })]),
    ],
    [
      "child-sigkill",
      () => {
        if (running()) child.kill("SIGKILL");
      },
    ],
    [
      "child-final-wait",
      async () => {
        if (running())
          await Promise.race([exited, pause(2_000, undefined, { ref: false })]);
        if (running()) throw new Error("owned-child-stop-deadline");
      },
    ],
  ]);
  if (failures.length) throw new TeardownFailure(failures);
}

export async function closeOwnedApi({ locks, proxies, child, logfile }) {
  const failures = await attemptAll([
    ...[...locks].map((release, i) => [`lock-release-${i}`, release]),
    ...proxies.map((proxy, i) => [`proxy-restore-${i}`, () => proxy.restore()]),
    ["api-child-stop", () => terminateOwnedChild(child)],
    ...[...proxies]
      .reverse()
      .map((proxy, i) => [`proxy-close-${i}`, () => proxy.close()]),
    ["private-log-close", () => logfile?.close()],
  ]);
  if (failures.length) throw new TeardownFailure(failures);
}
