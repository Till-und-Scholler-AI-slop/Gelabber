import { afterEach, describe, expect, it, vi } from "vitest";
import { addLayeredVideo, ViewerLayerController } from "./viewerLayers.ts";
import type { PeerConnection } from "./session.ts";
import type { MediaClientFrame } from "./media.ts";
const sources = [{ trackId: "native", userId: "user", kind: "s" as const }];
function report(received: number, lost: number, time: number) {
  return [
    {
      id: "in",
      type: "inbound-rtp",
      kind: "video",
      trackIdentifier: "native",
      packetsReceived: received,
      packetsLost: lost,
      timestamp: time,
    },
  ];
}
describe("viewer layer feedback", () => {
  it("reduces only the receiving source after real loss and recovers after four healthy intervals", () => {
    const controller = new ViewerLayerController(),
      frames: MediaClientFrame[] = [];
    const send = (f: MediaClientFrame) => frames.push(f);
    controller.update(report(100, 0, 1), sources, send, () => 1080);
    controller.update(report(200, 10, 2), sources, send, () => 1080);
    expect(frames.at(-1)).toEqual({
      op: "q",
      u: "user",
      k: "s",
      h: 1080,
      congested: true,
    });
    for (let n = 1; n <= 3; n++) {
      controller.update(
        report(200 + 100 * n, 10, 2 + n),
        sources,
        send,
        () => 240,
      );
      expect(frames.at(-1)).toMatchObject({ congested: true, h: 240 });
    }
    controller.update(report(600, 10, 6), sources, send, () => 240);
    expect(frames.at(-1)).toMatchObject({ congested: false });
  });
  it("does not classify missing, reset or repaired counters as network congestion", () => {
    const controller = new ViewerLayerController(),
      frames: MediaClientFrame[] = [];
    const send = (f: MediaClientFrame) => frames.push(f);
    controller.update(report(100, 10, 1), sources, send, () => 0);
    controller.update(report(200, 0, 2), sources, send, () => 0);
    expect(frames.at(-1)).toMatchObject({ congested: false, h: 0 });
    controller.update(
      [
        {
          id: "unknown",
          type: "inbound-rtp",
          kind: "video",
          trackIdentifier: "native",
        },
      ],
      sources,
      send,
      () => 0,
    );
    expect(frames.at(-1)).toMatchObject({ congested: false });
    controller.update(report(210, 1, 3), sources, send, () => 0);
    expect(frames.at(-1)).toMatchObject({ congested: false });
  });
  it("keeps newer loss evidence when native stats promises resolve out of order", () => {
    const controller = new ViewerLayerController(),
      frames: MediaClientFrame[] = [];
    const send = (f: MediaClientFrame) => frames.push(f);
    controller.update(report(100, 0, 1), sources, send, () => 360);
    controller.update(report(200, 10, 2), sources, send, () => 360);
    for (const stale of [report(150, 0, 1.5), report(200, 0, 2)]) {
      controller.update(stale, sources, send, () => 90);
      expect(frames.at(-1)).toMatchObject({ h: 90, congested: true });
    }
    controller.update(report(300, 10, 3), sources, send, () => 360);
    expect(frames.at(-1)).toMatchObject({ congested: true });
    controller.update(report(50, 0, 4), sources, send, () => 360);
    expect(frames.at(-1)).toMatchObject({ congested: false });
  });
  it("never invents a source for unrelated or retired receiver tracks", () => {
    const controller = new ViewerLayerController(),
      frames: MediaClientFrame[] = [];
    controller.update(report(100, 0, 1), [], (f) => frames.push(f));
    expect(frames).toEqual([]);
  });
});

describe("native layered publishing capability failure", () => {
  afterEach(() => vi.unstubAllGlobals());
  it("keeps the capture alive and removes a half-created sender before normal fallback", () => {
    vi.stubGlobal("RTCRtpSender", {
      getCapabilities: () => ({
        codecs: [{ mimeType: "video/VP8" }, { mimeType: "video/VP9" }],
      }),
    });
    const track = {
      kind: "video",
      stop: vi.fn(),
    } as unknown as MediaStreamTrack;
    const stream = {} as MediaStream;
    const sender = { track };
    const removeTrack = vi.fn();
    const addTransceiver = vi.fn((track: MediaStreamTrack, init: unknown) => {
      expect(track).toBe(sender.track);
      expect(init).toBeDefined();
      return {
        sender,
        setCodecPreferences: () => {
          throw Error("browser rejects preferences");
        },
      };
    });
    const pc = { addTransceiver, removeTrack } as unknown as PeerConnection;
    expect(addLayeredVideo(pc, track, stream)).toBeUndefined();
    expect(removeTrack).toHaveBeenCalledWith(sender);
    expect(track.stop).not.toHaveBeenCalled();
    expect(addTransceiver.mock.calls[0]?.[1]).toMatchObject({
      sendEncodings: [
        { rid: "q", scaleResolutionDownBy: 4 },
        { rid: "f", scaleResolutionDownBy: 1 },
      ],
    });
  });
  it("does not create an incompatible encoder when VP8 is absent", () => {
    vi.stubGlobal("RTCRtpSender", {
      getCapabilities: () => ({ codecs: [{ mimeType: "video/VP9" }] }),
    });
    const addTransceiver = vi.fn();
    expect(
      addLayeredVideo(
        { addTransceiver } as unknown as PeerConnection,
        {} as MediaStreamTrack,
        {} as MediaStream,
      ),
    ).toBeUndefined();
    expect(addTransceiver).not.toHaveBeenCalled();
  });
});
