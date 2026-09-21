import { describe, expect, it } from "vitest";

import {
  isOurTicket,
  openMediaSocket,
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

  it("adds Opus FEC and a voice bitrate when fmtp is missing", () => {
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
    expect(tuned).toContain(
      `maxaveragebitrate=${AUDIO_QUALITY.normal.bitrate}`,
    );
    expect(tuned).toContain("stereo=0");
    expect(opusMaxAverageBitrate(tuned)).toBe(AUDIO_QUALITY.normal.bitrate);
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
    expect(tuned).toContain("stereo=0");
    expect(tuned).toContain(
      `maxaveragebitrate=${AUDIO_QUALITY.normal.bitrate}`,
    );
    expect(tuned).toContain("minptime=10");
    expect(tuned).not.toContain("useinbandfec=0");
    expect(tuned).not.toContain("stereo=1");
    expect(tuned).not.toContain("maxaveragebitrate=24000");
  });
});

describe("media socket lifecycle", () => {
  it("clears isOpen after the transport closes or errors", () => {
    const opened: FakeSocket[] = [];
    class FakeSocket {
      listeners = new Map<string, Set<() => void>>();
      constructor(_url: string) {
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
