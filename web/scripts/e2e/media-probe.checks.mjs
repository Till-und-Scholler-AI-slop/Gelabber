/* global URL */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createContext, runInContext } from "node:vm";
import { instrument, sample } from "./probe.mjs";

test("concurrent claim fault holds only native Gateway publish, while SDP rejection still delegates to the browser", async () => {
  const sent = [],
    answers = [];
  class Peer {
    getSenders() {
      return [{ track: { kind: "video", readyState: "live" } }];
    }
    setRemoteDescription(description) {
      answers.push(description);
      return description.sdp === "valid"
        ? Promise.resolve()
        : Promise.reject(new Error("native SDP"));
    }
  }
  class Socket {
    constructor(url) {
      this.url = String(url);
    }
    addEventListener() {}
    send(data) {
      sent.push(JSON.parse(data));
    }
  }
  const window = {
    RTCPeerConnection: Peer,
    WebSocket: Socket,
    location: { href: "http://127.0.0.1/" },
    HTMLMediaElement: class {
      play() {
        return Promise.resolve();
      }
    },
  };
  const context = createContext({
    window,
    URL,
    navigator: { mediaDevices: { getUserMedia: async () => {} } },
  });
  runInContext(`(${instrument.toString()})({relay:false})`, context);
  window.__e2e.holdLiveClaims = true;
  const gateway = new window.WebSocket("ws://127.0.0.1/ws");
  const media = new window.WebSocket("ws://127.0.0.1/media/ws");
  const publish = { op: "sig", t: "p", k: "l" };
  gateway.send(JSON.stringify(publish));
  media.send(JSON.stringify(publish));
  gateway.send(JSON.stringify({ ...publish, t: "u" }));
  assert.equal(window.__e2e.heldLiveClaims.length, 1);
  assert.equal(sent.length, 2);
  window.__e2e.holdLiveClaims = false;
  window.__e2e.heldLiveClaims.splice(0)[0]();
  assert.deepEqual(sent.at(-1), publish);
  const peer = new window.RTCPeerConnection();
  window.__e2e.rejectNextVideoAnswer = true;
  await assert.rejects(
    peer.setRemoteDescription({ type: "answer", sdp: "valid" }),
  );
  await peer.setRemoteDescription({ type: "answer", sdp: "valid" });
  assert.equal(window.__e2e.rejectedSdp, 1);
  assert.equal(answers.length, 2);
  assert.notEqual(answers[0].sdp, "valid");
  assert.equal(answers[1].sdp, "valid");
});

test("room source observations require positive native RTP, deduplicate callbacks, and retain foreign-source failures", async () => {
  const tracks = ["a", "b", "foreign", "placeholder", "video"].map((id) => ({
    id,
    kind: id === "video" ? "video" : "audio",
    readyState: "live",
  }));
  const report = new Map(
    tracks.map((track, i) => [
      String(i),
      {
        type: "inbound-rtp",
        kind: track.kind,
        trackIdentifier: track.id,
        packetsReceived: track.id === "placeholder" ? 0 : 7,
      },
    ]),
  );
  const peer = {
    connectionState: "connected",
    iceConnectionState: "connected",
    getStats: async () => report,
    getTransceivers: () => [],
    getSenders: () => [],
    getReceivers: () => [],
    localDescription: null,
  };
  const incoming = tracks.map((track) => ({
    pc: peer,
    track,
    publisher: track.id === "video" ? "a" : track.id,
    sourceKind: track.kind === "video" ? "l" : "a",
  }));
  const state = {
    peers: [peer],
    incomingTracks: [...incoming, incoming[0]],
    expectedAudioPeer: peer,
    expectedAudioPublishers: ["a", "b"],
    expectedLivePublisher: "a",
    mediaElements: new Set(),
    heldTracks: [],
    heldLiveClaims: [],
    voiceRoster: [],
    sockets: [],
    captures: [],
    renderedVideos: new WeakMap(),
  };
  const context = createContext({
    window: { __e2e: state },
    document: { querySelectorAll: () => [] },
  });
  const read = () => runInContext(`(${sample.toString()})()`, context);
  const first = await read();
  assert.deepEqual([...first.roomAudio.perSource], [1, 1]);
  assert.equal(first.roomAudio.foreign, 1);
  assert.equal(first.watchVideoSources.selectedLive, 1);
  assert.equal(first.watchVideoSources.foreign, 0);
  incoming.at(-1).publisher = "foreign-video";
  const last = await read();
  assert.equal(last.watchVideoSources.selectedLive, 0);
  assert.equal(last.watchVideoSources.foreign, 1);
});
