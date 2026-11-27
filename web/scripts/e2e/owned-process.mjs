/* global process, console */
// The fault runner's own API and SFU hold fixed ports. If the runner ends
// without its teardown, they must not stay behind and block the next
// profile (GEL-457). An exit hook kills them when the runner's process
// ends in any way Node still sees; it does not run on SIGKILL, on SIGTERM
// or SIGINT sent to the runner alone, or when the kernel ends it for lack
// of memory. For those, each child leaves a PID file that names the child
// and the runner that started it. The next start ends the child only when
// that runner is dead and the child still runs this start's own binary;
// another runner that is still alive keeps everything it started.
import {
  lstat,
  readFile,
  readlink,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { setTimeout as pause } from "node:timers/promises";

// State and start time (field 22) of /proc/<pid>/stat, or null when gone.
async function processStat(pid) {
  const stat = await readFile(`/proc/${pid}/stat`, "utf8").catch(() => null);
  if (!stat) return null;
  const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
  return { state: fields[0], started: fields[19] };
}
// Running (not gone, not a zombie) and still the process that started then.
async function sameProcess(pid, started) {
  if (!Number.isInteger(pid) || pid <= 1 || typeof started !== "string")
    return false;
  const stat = await processStat(pid);
  return Boolean(stat) && stat.state !== "Z" && stat.started === started;
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
  const record = {
    pid,
    started: (await processStat(pid))?.started ?? null,
    exe: await realpath(binaryPath),
    runner: {
      pid: process.pid,
      started: (await processStat(process.pid))?.started ?? null,
    },
  };
  // Written private and new, then moved into place in one step.
  const draft = `${pidFile}.${randomUUID()}.new`;
  await writeFile(draft, JSON.stringify(record), { mode: 0o600, flag: "wx" });
  await rename(draft, pidFile);
}

export async function forgetOwned(pidFile) {
  await rm(pidFile, { force: true });
}

// What the next start does with what an earlier runner recorded in pidFile:
// - { removed: pid }: its runner was dead and the child, running this
//   start's binary, is ended now;
// - { blocked: reason }: a runner that is still alive owns the child, which
//   stays; this start must not take the port;
// - { kept: reason }: a file or a process that is not provably this start's
//   own leftover stays untouched;
// - {}: nothing recorded, or the child is gone already.
export async function removeLeftover(
  pidFile,
  binaryPath,
  { timeoutMs = 5_000 } = {},
) {
  const file = await lstat(pidFile).catch(() => null);
  if (!file) return {};
  if (
    !file.isFile() ||
    file.uid !== process.getuid() ||
    (file.mode & 0o777) !== 0o600
  )
    return { kept: "pid-file-not-private-to-this-user" };
  let record;
  try {
    record = JSON.parse(await readFile(pidFile, "utf8"));
  } catch {
    return { kept: "pid-file-unreadable" };
  }
  const { pid, started, runner } = record ?? {};
  if (await sameProcess(runner?.pid, runner?.started))
    return { blocked: "live-fault-run-owns-the-ports" };
  if (!(await sameProcess(pid, started))) {
    await forgetOwned(pidFile);
    return {};
  }
  // After a rebuild the old process's binary reads "<path> (deleted)".
  const exe = await readlink(`/proc/${pid}/exe`).catch(() => null);
  if (exe !== (await realpath(binaryPath)))
    return { kept: "leftover-binary-is-not-this-starts-binary" };
  process.kill(pid, "SIGKILL");
  const deadline = Date.now() + timeoutMs;
  while (await sameProcess(pid, started)) {
    if (Date.now() >= deadline) throw new Error("owned-leftover-did-not-end");
    await pause(50);
  }
  await forgetOwned(pidFile);
  return { removed: pid };
}

// A start the leftover check stops, with the reason its BLOCKED row names.
export class OwnedPortsBlocked extends Error {
  constructor(reason) {
    super(reason);
    this.name = "OwnedPortsBlocked";
  }
}

// What a start does before it takes the ports of its `name` (API or SFU):
// ends a dead runner's leftover and says so, stops when a live run owns the
// ports, and says why a leftover it may not end stays.
export async function clearLeftover(pidFile, binaryPath, name) {
  const leftover = await removeLeftover(pidFile, binaryPath);
  if (leftover.blocked) throw new OwnedPortsBlocked(leftover.blocked);
  if (leftover.removed)
    console.log(
      `Removed the owned ${name} (pid ${leftover.removed}) a dead runner left behind`,
    );
  if (leftover.kept)
    console.log(`Leftover owned ${name} not removed: ${leftover.kept}`);
  return leftover;
}
// The port probe's failure, named after a leftover that had to stay.
export function portTaken(leftover, error) {
  return leftover.kept
    ? new OwnedPortsBlocked(`port-taken-${leftover.kept}`)
    : error;
}
