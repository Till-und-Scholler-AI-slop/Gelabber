/* global URL */
// The actual voice-only scenario from media-extra.mjs against fake browsers:
// each actor's snapshots follow a script of samples, the last one repeating.
import vm from "node:vm";
import { readFile } from "node:fs/promises";
import { check, CheckFailure } from "./harness.mjs";
import * as native from "./native-evaluate.mjs";
import * as teardown from "./teardown.mjs";
import { activePeers } from "./media.mjs";

/** One snapshot: `packets` received, `plays` playback paths of the one
 * expected source (or `perSource`, one count per source), `foreign` paths
 * of any other. */
export function sample({
  packets,
  plays = 1,
  perSource = [plays],
  foreign = 0,
}) {
  return {
    peers: [
      {
        connection: "connected",
        audioSenders: [{ live: true, enabled: true }],
        inbound: [{ kind: "audio", packets }],
        outbound: [{ kind: "audio", packets }],
        selected: [{ state: "succeeded", local: "host", remote: "host" }],
      },
    ],
    roomAudio: { perSource, foreign },
    duplicateAudioPlaybackTracks: 0,
    displayCalls: 0,
    cameraCalls: 0,
  };
}

/** Runs the scenario with `owner` and `member`, each a list of samples. */
export async function voiceOnlyControl({ owner, member }) {
  const scripts = new Map();
  const actor = (name, samples) => {
    const who = { id: name, label: name };
    scripts.set(who, { samples, next: 0 });
    return who;
  };
  const f = {
    owner: actor("owner", owner),
    member: actor("member", member),
    watcher: actor("watcher", [sample({ packets: 0 })]),
  };
  const harness = {
    check,
    CheckFailure,
    click: async () => {},
    liveTiles: () => ({}),
    snapshot: async (who) => {
      const script = scripts.get(who);
      const at = Math.min(script.next++, script.samples.length - 1);
      return script.samples[at];
    },
    // Samples instead of time: at most as many polls as a script is long.
    until: async (probe, accept, code) => {
      let last;
      for (let poll = 0; poll < 10; poll++) {
        last = await probe();
        if (accept(last)) return last;
      }
      throw new CheckFailure(code, { last });
    },
  };
  const mocks = {
    "./harness.mjs": harness,
    "./native-evaluate.mjs": { ...native, nativeEvaluate: async () => {} },
    "./teardown.mjs": teardown,
    "./media.mjs": {
      activePeers,
      armPlaybackRetry: () => {},
      progress: async () => ({}),
      // Its own checks are in harness.checks.mjs; here the path is settled.
      waitForPath: async () => [],
    },
  };
  const context = vm.createContext({}),
    url = new URL("./media-extra.mjs", import.meta.url);
  const source = new vm.SourceTextModule(await readFile(url, "utf8"), {
    context,
    identifier: url.href,
  });
  await source.link(async (name) => {
    const exports = mocks[name] ?? (await import(name));
    return new vm.SyntheticModule(
      Object.keys(exports),
      function () {
        for (const [key, value] of Object.entries(exports))
          this.setExport(key, value);
      },
      { context },
    );
  });
  await source.evaluate();
  let result, error;
  const h = {
    run: async (id, _predecessors, task) => {
      if (id !== "voice-only-duplex-single-source") return;
      try {
        result = await task();
      } catch (e) {
        error = e;
      }
    },
  };
  await source.namespace.mediaExtraScenarios(h, f, {
    begin: async () => {},
    reset: async () => {},
    options: { budget: 5_000, relay: false },
  });
  return { result, error };
}
