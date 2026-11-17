/* global process, console */
// Phase labels and result lines shared by the load scenarios. sampler.py
// reads the current phase from the same file on every sample.
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const PHASE_FILE =
  process.env.GELABBER_LOAD_PHASE_FILE ?? join(tmpdir(), "gelabber-load-phase");
export const LABEL_FILE =
  process.env.GELABBER_LOAD_LABEL_FILE ??
  join(tmpdir(), "gelabber-load-labels.json");

export function phase(name) {
  writeFileSync(PHASE_FILE, name);
  console.log(new Date().toISOString(), "phase", name);
}

export function resultLog(path) {
  return (record) =>
    appendFileSync(
      path,
      JSON.stringify({ t: Date.now() / 1000, ...record }) + "\n",
    );
}

/** Names a browser's process tree for sampler.py (root pid -> label). */
export function labelProcess(pid, name) {
  let labels = {};
  try {
    labels = JSON.parse(readFileSync(LABEL_FILE, "utf8"));
  } catch {
    // first label
  }
  if (name === null) delete labels[pid];
  else labels[pid] = name;
  writeFileSync(LABEL_FILE, JSON.stringify(labels));
}
