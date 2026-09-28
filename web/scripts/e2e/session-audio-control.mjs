/* global URL */
// Complete actual scenario source; only native/UI/observation infrastructure is synthetic.
import vm from "node:vm";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { check, CheckFailure } from "./harness.mjs";
import * as native from "./native-evaluate.mjs";
import * as teardown from "./teardown.mjs";
import { activePeers } from "./media.mjs";
function data({ sender = true, enabled = true, muted = false } = {}) {
  return {
    peers: [
      {
        connection: "connected",
        audioSenders: sender ? [{ live: true, enabled }] : [],
        inbound: [{ kind: "audio", packets: 20 }],
        outbound: [
          { kind: "audio", packets: 20 },
          { kind: "video", frames: 20 },
        ],
      },
    ],
    playback: Array.from({ length: 3 }, () => ({
      kind: "audio",
      audioTracks: 1,
      paused: false,
      muted,
      volume: 0.35,
    })),
    roomAudio: { perSource: [1, 1], foreign: 0 },
    watchVideoSources: { selectedLive: 1, foreign: 0 },
    duplicateAudioPlaybackTracks: 0,
    micCalls: 0,
    playRejected: 1,
  };
}
export async function sessionAudioControl({
  lostMicAt,
  replaceSender = false,
  replaceTrack = false,
  finalRestoreError = false,
  otherCloseError = false,
  micReadyAfterPoll = false,
  micNeverConnected = false,
  micNoOutboundRtp = false,
  otherPeerOnlyRtp = false,
} = {}) {
  const actions = [];
  let finishedVolume = false,
    result,
    error;
  const locator = {
    click: async () => {},
    press: async () => {},
    count: async () => 1,
    first() {
      return this;
    },
  };
  function actor(name) {
    const microphone = { kind: "audio", readyState: "live", enabled: true };
    const sender = { track: microphone };
    const state = {
      nativeSenders: [sender],
      peers: [
        { connectionState: "connected", getSenders: () => state.nativeSenders },
      ],
    };
    const context = vm.createContext({ window: { __e2e: state } });
    return {
      id: name,
      sample: data(),
      state,
      sender,
      microphone,
      page: {
        locator: () => locator,
        getByRole: () => locator,
        isClosed: () => false,
        evaluate: async (fn, arg) => {
          if (name === "watcher" && finishedVolume && finalRestoreError) {
            actions.push("final-restore-rejected");
            throw new Error("PRIVATE-native-restore-error");
          }
          return vm.runInContext(`(${fn.toString()})`, context)(arg);
        },
        close: async () => {},
      },
      context: {
        close: async () => {
          actions.push(name + ".context.close");
          if (name === "other" && otherCloseError)
            throw new Error("PRIVATE-other-close-error");
        },
      },
    };
  }
  const f = {
    owner: actor("owner"),
    member: actor("member"),
    watcher: actor("watcher"),
    join: async () => {},
    otherPath: "/s/control/c/other",
  };
  const other = actor("other");
  if (micReadyAfterPoll || micNeverConnected)
    f.watcher.state.peers[0].connectionState = "connecting";
  if (micNoOutboundRtp || otherPeerOnlyRtp)
    f.watcher.sample.peers[0].outbound[0].packets = 0;
  if (otherPeerOnlyRtp) {
    f.watcher.state.peers.unshift({
      connectionState: "connected",
      getSenders: () => [],
    });
    f.watcher.sample.peers.unshift({
      connection: "connected",
      audioSenders: [],
      inbound: [],
      outbound: [{ kind: "audio", packets: 20 }],
    });
  }
  const harness = {
    check,
    CheckFailure,
    click: async () => {},
    snapshot: async (actor) => actor.sample,
    observe: async (_ms, probe) => probe(),
    until: async (probe, accept, code, budget) => {
      if (code === "fixture-session-positive-microphone-not-ready") {
        assert.equal(budget, 5_000);
        for (let poll = 0; poll < 2; poll++) {
          const measured = await probe();
          actions.push("microphone-fixture-poll");
          if (accept(measured)) return measured;
          if (micReadyAfterPoll)
            f.watcher.state.peers[0].connectionState = "connected";
        }
        throw new CheckFailure(code, { last: await probe() });
      }
      const phase =
        code === "session-mute-did-not-disable-mic"
          ? "mute"
          : code === "session-deafen-did-not-mute-every-path"
            ? "deafen"
            : code === "session-undeafen-did-not-restore-every-path"
              ? "undeafen"
              : null;
      if (phase) {
        const enabled = phase === "undeafen",
          lost = lostMicAt === phase;
        f.watcher.microphone.enabled = enabled;
        if (lost) f.watcher.state.nativeSenders = [];
        if (phase === "undeafen" && replaceSender)
          f.watcher.state.nativeSenders = [{ track: f.watcher.microphone }];
        if (phase === "undeafen" && replaceTrack)
          f.watcher.sender.track = {
            kind: "audio",
            readyState: "live",
            enabled: true,
          };
        f.watcher.sample = data({
          sender: !lost,
          enabled,
          muted: phase === "deafen",
        });
      }
      const sample = await probe();
      check(accept(sample), code, { last: sample });
      if (code === "session-volume-not-applied-to-every-path")
        finishedVolume = true;
      return sample;
    },
  };
  const mocks = {
    "./harness.mjs": harness,
    "./native-evaluate.mjs": native,
    "./teardown.mjs": teardown,
    "./media.mjs": {
      activePeers,
      progress: async () => ({ syntheticPositiveFixture: true }),
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
  const h = {
    actor: async () => other,
    run: async (id, _predecessors, task) => {
      if (id === "session-audio-mute-deafen-volume-playback-retry") {
        try {
          result = await task();
        } catch (e) {
          error = e;
        }
      }
    },
  };
  await source.namespace.mediaExtraScenarios(h, f, {
    begin: async () => {},
    reset: async () => {},
    options: {},
  });
  return { result, error, actions };
}
