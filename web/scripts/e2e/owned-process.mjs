/* global process */
// The fault runner's own API and SFU hold fixed ports. If the runner ends
// without its teardown, they must not stay behind and block the next
// profile (GEL-457). An exit hook kills them when the runner's process
// ends in any way Node still sees; for the rest (SIGKILL, a signal), each
// child leaves a PID file that the next start reads, and that start removes
// only a process whose PID, start time and binary all match the file.
import { readFile, readlink, realpath, rm, writeFile } from "node:fs/promises";
import { setTimeout as pause } from "node:timers/promises";

// State and start time (field 22) of /proc/<pid>/stat, or null when gone.
async function processStat(pid) {
  const stat = await readFile(`/proc/${pid}/stat`, "utf8").catch(() => null);
  if (!stat) return null;
  const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
  return { state: fields[0], started: fields[19] };
}

export function killOnExit(child) {
  const kill = () => {
    if (child.exitCode === null && child.signalCode === null)
      child.kill("SIGKILL");
  };
  process.on("exit", kill);
  return () => process.off("exit", kill);
}

export async function recordOwned(pidFile, pid, binaryPath) {
  const stat = await processStat(pid);
  await writeFile(
    pidFile,
    JSON.stringify({
      pid,
      started: stat?.started ?? null,
      exe: await realpath(binaryPath),
    }),
    { mode: 0o600 },
  );
}

export async function forgetOwned(pidFile) {
  await rm(pidFile, { force: true });
}

// Ends the process an earlier runner recorded in pidFile, if it still runs.
// Returns its PID, or null when there was none (or the PID now belongs to
// another process, which stays untouched).
export async function removeLeftover(pidFile, { timeoutMs = 5_000 } = {}) {
  let record;
  try {
    record = JSON.parse(await readFile(pidFile, "utf8"));
  } catch {
    return null;
  }
  const { pid, started, exe } = record ?? {};
  const stat = Number.isInteger(pid) && pid > 1 ? await processStat(pid) : null;
  const exeNow = stat
    ? await readlink(`/proc/${pid}/exe`).catch(() => null)
    : null;
  if (
    !stat ||
    stat.state === "Z" ||
    stat.started !== started ||
    exeNow !== exe
  ) {
    await forgetOwned(pidFile);
    return null;
  }
  process.kill(pid, "SIGKILL");
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const now = await processStat(pid);
    if (!now || now.state === "Z" || now.started !== started) break;
    if (Date.now() >= deadline) throw new Error("owned-leftover-did-not-end");
    await pause(50);
  }
  await forgetOwned(pidFile);
  return pid;
}
