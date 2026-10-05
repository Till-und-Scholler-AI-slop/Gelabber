import { describe, expect, it } from "vitest";

import {
  isOurTicket,
  openMediaSocket,
  publishedTrackIds,
  opusMaxAverageBitrate,
  tuneAudioSdp,
} from "./media.ts";
import { MediaPeer } from "./mediaPeer.ts";
import { AUDIO_QUALITY } from "./settings.ts";

describe("media ticket shape", () => {
  it("accepts our 12-char alphabet and rejects product tokens", () => {
    expect(isOurTicket("abcdefghjkmn")).toBe(true);
    expect(isOurTicket("livekit_jwt_xxx")).toBe(false);
    expect(isOurTicket("SHORT")).toBe(false);
    expect(isOurTicket("abcdefghi0mn")).toBe(false);
  });

  it("adds Opus FEC without an application bitrate cap when fmtp is missing", () => {
    const sdp = [
      "v=0",
      "m=audio 9 UDP/TLS/RTP/SAVPF 111",
      "a=rtpmap:111 opus/48000/2",
      "a=mid:0",
      "",
    ].join("\r\n");
    const tuned = tuneAudioSdp(sdp);
    expect(tuned).toContain("a=fmtp:111 ");
    expect(tuned).toContain("useinbandfec=1");
    expect(tuned).not.toContain("maxaveragebitrate");
    expect(tuned).toContain("stereo=1");
    expect(opusMaxAverageBitrate(tuned)).toBeNull();
  });

  it("writes the chosen quality bitrate onto Opus fmtp", () => {
    const sdp = [
      "m=audio 9 UDP/TLS/RTP/SAVPF 111",
      "a=rtpmap:111 opus/48000/2",
      "",
    ].join("\r\n");
    expect(
      opusMaxAverageBitrate(tuneAudioSdp(sdp, AUDIO_QUALITY.phone.bitrate)),
    ).toBe(24_000);
    expect(
      opusMaxAverageBitrate(tuneAudioSdp(sdp, AUDIO_QUALITY.high.bitrate)),
    ).toBe(128_000);
  });

  it("raises an existing Opus fmtp to FEC instead of duplicating the line", () => {
    const sdp = [
      "a=rtpmap:111 opus/48000/2",
      "a=fmtp:111 minptime=10;useinbandfec=0",
      "",
    ].join("\r\n");
    const tuned = tuneAudioSdp(sdp);
    expect(tuned.match(/a=fmtp:111 /g)).toHaveLength(1);
    expect(tuned).toContain("useinbandfec=1");
    expect(tuned).toContain("minptime=10");
  });

  it("merges Chromium's rtpmap / rtcp-fb / fmtp into one fmtp line", () => {
    const sdp = [
      "v=0",
      "m=audio 9 UDP/TLS/RTP/SAVPF 111",
      "a=rtpmap:111 opus/48000/2",
      "a=rtcp-fb:111 transport-cc",
      "a=fmtp:111 minptime=10;useinbandfec=0;stereo=1;maxaveragebitrate=24000",
      "a=mid:0",
      "",
    ].join("\r\n");
    const tuned = tuneAudioSdp(sdp);
    expect(tuned.match(/a=fmtp:111 /g)).toHaveLength(1);
    expect(tuned).toContain("a=rtcp-fb:111 transport-cc");
    expect(tuned).toContain("useinbandfec=1");
    expect(tuned).toContain("stereo=1");
    expect(tuned).not.toContain("maxaveragebitrate");
    expect(tuned).toContain("minptime=10");
    expect(tuned).not.toContain("useinbandfec=0");
    expect(tuned).toContain("sprop-stereo=0");
    expect(tuned).not.toContain("maxaveragebitrate=24000");
  });
});

describe("media socket lifecycle", () => {
  it("clears isOpen after the transport closes or errors", () => {
    const opened: FakeSocket[] = [];
    class FakeSocket {
      listeners = new Map<string, Set<() => void>>();
      constructor() {
        opened.push(this);
      }
      addEventListener(type: string, fn: () => void) {
        const set = this.listeners.get(type) ?? new Set();
        set.add(fn);
        this.listeners.set(type, set);
      }
      send() {}
      close() {
        this.emit("close");
      }
      emit(type: string) {
        for (const fn of this.listeners.get(type) ?? []) fn();
      }
    }
    const previous = globalThis.WebSocket;
    globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket;
    try {
      const first = openMediaSocket("ws://localhost/media/ws");
      const peer = new MediaPeer();
      peer.bind(first, () => {});
      expect(peer.isOpen()).toBe(true);
      opened[0]?.emit("open");
      expect(peer.isOpen()).toBe(true);
      opened[0]?.emit("close");
      expect(peer.isOpen()).toBe(false);

      const second = openMediaSocket("ws://localhost/media/ws");
      const again = new MediaPeer();
      again.bind(second, () => {});
      opened[1]?.emit("error");
      expect(again.isOpen()).toBe(false);
    } finally {
      globalThis.WebSocket = previous;
    }
  });
});

describe("publisher SDP identity", () => {
  it("accepts port-zero bundle-only senders only with a live BUNDLE transport", () => {
    const sdp =
      "v=0\r\na=group:BUNDLE audio camera screen rejected\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:audio\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:camera\r\na=bundle-only\r\na=sendrecv\r\na=msid:camera-stream camera-track\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:screen\r\na=bundle-only\r\na=sendonly\r\na=msid:screen-stream screen-track\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:rejected\r\na=msid:removed-stream removed-track\r\n";
    expect([...publishedTrackIds(sdp)]).toEqual([
      ["camera", "camera-track"],
      ["screen", "screen-track"],
    ]);
    expect([
      ...publishedTrackIds(
        sdp.replace("a=group:BUNDLE audio camera screen rejected\r\n", ""),
      ),
    ]).toEqual([]);
    expect([
      ...publishedTrackIds(sdp.replace("m=audio 9", "m=audio 0")),
    ]).toEqual([]);
    expect([
      ...publishedTrackIds(sdp.replace("a=sendrecv", "a=recvonly")),
    ]).toEqual([["screen", "screen-track"]]);
  });
  it("maps active video MID to its MSID, not its capture track ID", () => {
    const sdp =
      "v=0\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:2\r\na=msid:new-stream retained-track\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=mid:3\r\na=recvonly\r\na=msid:old-stream stopped-track\r\n";
    expect([...publishedTrackIds(sdp)]).toEqual([["2", "retained-track"]]);
  });
  it("supports legacy SSRC MSID and ignores rejected video", () => {
    const sdp =
      "v=0\nm=video 9 UDP/TLS/RTP/SAVPF 96\na=mid:screen\na=ssrc:123 msid:stream screen-track\nm=video 0 UDP/TLS/RTP/SAVPF 96\na=mid:removed\na=msid:stream removed-track\n";
    expect([...publishedTrackIds(sdp)]).toEqual([["screen", "screen-track"]]);
  });
});

describe("source music SDP", () => {
  const section = (mid: string, track: string) => [
    "m=audio 9 UDP/TLS/RTP/SAVPF 111",
    `a=mid:${mid}`,
    "a=rtpmap:111 opus/48000/2",
    "a=fmtp:111 useinbandfec=0;usedtx=1;stereo=0;maxaveragebitrate=24000",
    `a=msid:capture ${track}`,
  ];
  it("tunes screen/Live music separately from microphone speech, including a reused MID", () => {
    const sdp = [
      "v=0",
      ...section("0", "mic"),
      ...section("1", "source"),
      ...section("2", "retained"),
      "",
    ].join("\r\n");
    const tuned = tuneAudioSdp(
      sdp,
      24_000,
      new Set(["source"]),
      new Set(["2"]),
    );
    const sections = tuned.split(/\r\nm=/).slice(1);
    expect(sections[0]).toContain("usedtx=1;stereo=1;maxaveragebitrate=24000");
    for (const source of sections.slice(1)) {
      expect(source).toContain("useinbandfec=1");
      expect(source).toContain("usedtx=0;stereo=1");
      expect(source).not.toContain("maxaveragebitrate");
      expect(source).toContain("sprop-stereo=1");
    }
    expect(publishedTrackIds(tuned)).toEqual(
      new Map([
        ["0", "mic"],
        ["1", "source"],
        ["2", "retained"],
      ]),
    );
  });
  it("recognizes forwarded audio tags within their shared parent video MSID", () => {
    const sdp = ["v=0", ...section("0", "u-bob:la-789"), ""]
      .join("\r\n")
      .replace("a=msid:capture", "a=msid:u-bob:l");
    const tuned = tuneAudioSdp(sdp, 64_000);
    expect(tuned).toContain("usedtx=0");
    expect(tuned).toContain("stereo=1");
    expect(tuned).not.toContain("maxaveragebitrate");
  });
});
