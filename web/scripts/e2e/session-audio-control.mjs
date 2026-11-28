/* global URL, setTimeout, clearTimeout, window */
// Complete actual scenario source; only native/UI/observation infrastructure is synthetic.
import vm from "node:vm";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { check, CheckFailure } from "./harness.mjs";
import * as native from "./native-evaluate.mjs";
import * as teardown from "./teardown.mjs";
import { activePeers } from "./media.mjs";
// The call's two audio paths: the owner's and the member's microphone.
function data({
  sender = true,
  enabled = true,
  muted = false,
  spared = false,
} = {}) {
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
    playback: Array.from({ length: 2 }, (_, i) => ({
      kind: "audio",
      audioTracks: 1,
      paused: false,
      muted: muted && !(spared && i === 0),
      volume: 1,
    })),
    roomAudio: { perSource: [1, 1], foreign: 0 },
    watchVideoSources: { selectedLive: 1, foreign: 0 },
    duplicateAudioPlaybackTracks: 0,
    // The watcher is in the call: one microphone capture, none for watching.
    micCalls: 1,
    playRejected: 1,
    audioPlayStarted: 0,
  };
}
export async function sessionAudioControl({
  lostMicAt,
  replaceSender = false,
  replaceTrack = false,
  finalRestoreError = false,
  retryStartsNothing = false,
  noticeStays = false,
  muteDoesNothing = false,
  deafenSparesOne = false,
  undeafenSparesOne = false,
  volumeSparesOne = false,
  micReadyAfterPoll = false,
  micNeverConnected = false,
  micNoOutboundRtp = false,
  otherPeerOnlyRtp = false,
  samePeerOtherSenderOnlyRtp = false,
} = {}) {
  const actions = [];
  let finishedVolume = false,
    retried = false,
    volumeSteps = 100,
    result,
    error;
  // The only control the scenario counts is the retry, gone once clicked;
  // the only one it presses keys on is the call volume, 1 % a step.
  const locator = {
    click: async () => {
      retried = true;
    },
    press: async (key) => {
      if (key === "Home") volumeSteps = 0;
      if (key === "ArrowRight") volumeSteps++;
    },
    count: async () => (retried && !noticeStays ? 0 : 1),
    first() {
      return this;
    },
    getByRole() {
      return this;
    },
  };
  function actor(name) {
    const microphone = { kind: "audio", readyState: "live", enabled: true };
    let senderStatsCalls = 0;
    const sender = {
      track: microphone,
      getStats: async () => {
        senderStatsCalls++;
        return new Map([
          [
            "owned-sender-audio-rtp",
            {
              type: "outbound-rtp",
              kind: "audio",
              packetsSent: sender.packetsSent,
            },
          ],
        ]);
      },
      packetsSent: 20,
    };
    const state = {
      nativeSenders: [sender],
      peers: [
        { connectionState: "connected", getSenders: () => state.nativeSenders },
      ],
    };
    const context = vm.createContext({
      window: { __e2e: state },
      setTimeout,
      clearTimeout,
    });
    return {
      id: name,
      sample: data(),
      state,
      sender,
      microphone,
      get senderStatsCalls() {
        return senderStatsCalls;
      },
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
      context: { close: async () => {} },
    };
  }
  const f = {
    owner: actor("owner"),
    member: actor("member"),
    watcher: actor("watcher"),
  };
  if (micReadyAfterPoll || micNeverConnected)
    f.watcher.state.peers[0].connectionState = "connecting";
  if (micNoOutboundRtp || otherPeerOnlyRtp)
    f.watcher.sample.peers[0].outbound[0].packets = 0;
  if (micNoOutboundRtp || otherPeerOnlyRtp || samePeerOtherSenderOnlyRtp)
    f.watcher.sender.packetsSent = 0;
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
    liveTiles: () => locator,
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
        const enabled =
            phase === "undeafen" || (phase === "mute" && muteDoesNothing),
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
      if (
        code === "audio-click-retry-did-not-play-every-path" &&
        !retryStartsNothing
      )
        f.watcher.sample.audioPlayStarted += 2;
      if (phase === "deafen" && deafenSparesOne)
        f.watcher.sample = data({
          enabled: false,
          muted: true,
          spared: true,
        });
      if (phase === "undeafen" && undeafenSparesOne)
        f.watcher.sample.playback[0].muted = true;
      if (code === "session-volume-not-applied-to-every-path")
        f.watcher.sample.playback.forEach((path, i) => {
          path.volume = volumeSparesOne && i === 0 ? 1 : volumeSteps / 100;
        });
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
      watchSource: async () => {},
      activePeers,
      armPlaybackRetry: function armPlaybackRetry() {
        window.__e2e.retryArmed = true;
      },
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
  return {
    result,
    error,
    actions,
    watcherState: f.watcher.state,
    watcherSenderStatsCalls: f.watcher.senderStatsCalls,
  };
}
