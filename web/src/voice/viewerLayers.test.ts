import { describe, expect, it, vi } from "vitest";
import { ViewerLayerController, type ViewerSource } from "./viewerLayers.ts";
import type { MediaRequests } from "./media.ts";
import type { StatsEntry } from "./diagnostics.ts";
import { MediaPeer } from "./mediaPeer.ts";
import type {
  MediaClientFrame,
  MediaServerFrame,
  MediaSocket,
} from "./media.ts";
const a: ViewerSource = {
  consumerId: "a",
  generation: "gen-a",
  trackId: "track-a",
  ssrc: 101,
};
const b: ViewerSource = {
  consumerId: "b",
  generation: "gen-b",
  trackId: "track-b",
  ssrc: 202,
};
const row = (
  ssrc: number,
  timestamp: number,
  packetsReceived: number,
  packetsLost: number,
): StatsEntry => ({
  id: String(ssrc),
  type: "inbound-rtp",
  kind: "video",
  ssrc,
  timestamp,
  packetsReceived,
  packetsLost,
});
describe("per-consumer viewer preference", () => {
  it("delivers hints for 65 consumers within the RPC budget and retries rejected hints", async () => {
    const peer = new MediaPeer();
    const sent: MediaClientFrame[] = [];
    let emit!: (frame: MediaServerFrame) => void;
    let outstanding = 0,
      peak = 0,
      rejectOnce = true;
    const socket: MediaSocket = {
      send: (frame) => {
        sent.push(frame);
        peak = Math.max(peak, ++outstanding);
        queueMicrotask(() => {
          outstanding--;
          if (
            frame.op === "q" &&
            frame.consumerId === "consumer-64" &&
            rejectOnce
          ) {
            rejectOnce = false;
            emit({ op: "err", id: frame.id, e: "unavailable" });
          } else emit({ op: "result", id: frame.id, data: {} });
        });
      },
      close: () => {},
      onFrame: (callback) => {
        emit = callback;
        return () => {};
      },
      onClose: () => () => {},
    };
    peer.bind(socket, () => {});
    peer.accepted = true;
    const sources = Array.from({ length: 65 }, (_, index) => ({
      consumerId: `consumer-${index}`,
      generation: "generation",
      trackId: `track-${index}`,
    }));
    const controller = new ViewerLayerController();
    const send = (frame: MediaRequests["q"]) => peer.request("q", frame);
    try {
      await controller.update([], sources, send, () => 1080);
      expect(sent).toHaveLength(65);
      expect(peak).toBeLessThanOrEqual(8);
      await controller.update([], sources, send, () => 1080);
      expect(sent).toHaveLength(66);
      expect(sent.at(-1)).toMatchObject({ op: "q", consumerId: "consumer-64" });
      await controller.update([], sources, send, () => 1080);
      expect(sent).toHaveLength(66);
    } finally {
      peer.close();
    }
  });
  it("binds loss to exact consumer SSRC and keeps other consumers independent", async () => {
    const c = new ViewerLayerController(),
      requests: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => requests.push(q);
    await c.update(
      [row(101, 0, 100, 0), row(202, 0, 100, 0)],
      [a, b],
      send,
      (id) => (id === a.trackId ? 180 : 1080),
    );
    await c.update(
      [row(101, 1000, 120, 10), row(202, 1000, 150, 0)],
      [a, b],
      send,
      (id) => (id === a.trackId ? 180 : 1080),
    );
    expect(requests).toHaveLength(3);
    expect(requests.at(-1)).toEqual({
      consumerId: "a",
      generation: "gen-a",
      h: 180,
      congested: true,
    });
  });
  it("deduplicates unchanged hints, refreshes slowly, and forgets retired generations", async () => {
    const clock = vi.spyOn(Date, "now").mockReturnValue(0);
    try {
      const controller = new ViewerLayerController();
      const send = vi.fn();
      await controller.update([], [a], send, () => 720);
      for (let second = 2; second < 30; second += 2) {
        clock.mockReturnValue(second * 1000);
        await controller.update([], [a], send, () => 720);
      }
      expect(send).toHaveBeenCalledTimes(1);
      clock.mockReturnValue(30_000);
      await controller.update([], [a], send, () => 720);
      expect(send).toHaveBeenCalledTimes(2);
      await controller.update([], [a], send, () => 90);
      expect(send).toHaveBeenCalledTimes(3);
      await controller.update([], [], send);
      await controller.update([], [a], send, () => 90);
      expect(send).toHaveBeenCalledTimes(4);
    } finally {
      clock.mockRestore();
    }
  });
  it("requires four healthy intervals to restore a congested consumer", async () => {
    const c = new ViewerLayerController(),
      out: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => out.push(q);
    await c.update([row(101, 0, 100, 0)], [a], send, () => 720);
    await c.update([row(101, 1, 120, 10)], [a], send, () => 720);
    for (let i = 1; i <= 3; i++)
      await c.update([row(101, i + 1, 120 + i * 30, 10)], [a], send, () => 720);
    expect(out.at(-1)?.congested).toBe(true);
    await c.update([row(101, 5, 240, 10)], [a], send, () => 720);
    expect(out.at(-1)?.congested).toBe(false);
  });
  it("retains qualified congestion through stale, missing, malformed and reset counters", async () => {
    const c = new ViewerLayerController(),
      out: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => out.push(q);
    await c.update([row(101, 10, 100, 0)], [a], send, () => 180);
    await c.update([row(101, 20, 120, 10)], [a], send, () => 180);
    for (const rows of [
      [row(101, 15, 999, 10)],
      [],
      [row(101, 30, NaN, 10)],
      [row(101, 40, 1, 0)],
    ]) {
      await c.update(rows, [a], send, () => 1080);
      expect(out.at(-1)).toMatchObject({ h: 1080, congested: true });
    }
  });
  it("does not carry loss evidence into a replacement subscription generation", async () => {
    const c = new ViewerLayerController(),
      out: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => out.push(q);
    await c.update([row(101, 0, 100, 0)], [a], send, () => 180);
    await c.update([row(101, 1, 120, 10)], [a], send, () => 180);
    await c.update([], [{ ...a, generation: "replacement" }], send, () => 1080);
    expect(out.at(-1)).toMatchObject({
      generation: "replacement",
      congested: false,
    });
  });
});
