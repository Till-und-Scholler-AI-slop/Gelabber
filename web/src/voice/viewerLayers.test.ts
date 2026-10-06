import { describe, expect, it, vi } from "vitest";
import { ViewerLayerController, type ViewerSource } from "./viewerLayers.ts";
import type { MediaRequests } from "./media.ts";
import type { StatsEntry } from "./diagnostics.ts";
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
  it("binds loss to exact consumer SSRC and keeps other consumers independent", () => {
    const c = new ViewerLayerController(),
      requests: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => requests.push(q);
    c.update([row(101, 0, 100, 0), row(202, 0, 100, 0)], [a, b], send, (id) =>
      id === a.trackId ? 180 : 1080,
    );
    c.update(
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
  it("deduplicates unchanged hints, refreshes slowly, and forgets retired generations", () => {
    const clock = vi.spyOn(Date, "now").mockReturnValue(0);
    try {
      const controller = new ViewerLayerController();
      const send = vi.fn();
      controller.update([], [a], send, () => 720);
      for (let second = 2; second < 30; second += 2) {
        clock.mockReturnValue(second * 1000);
        controller.update([], [a], send, () => 720);
      }
      expect(send).toHaveBeenCalledTimes(1);
      clock.mockReturnValue(30_000);
      controller.update([], [a], send, () => 720);
      expect(send).toHaveBeenCalledTimes(2);
      controller.update([], [a], send, () => 90);
      expect(send).toHaveBeenCalledTimes(3);
      controller.update([], [], send);
      controller.update([], [a], send, () => 90);
      expect(send).toHaveBeenCalledTimes(4);
    } finally {
      clock.mockRestore();
    }
  });
  it("requires four healthy intervals to restore a congested consumer", () => {
    const c = new ViewerLayerController(),
      out: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => out.push(q);
    c.update([row(101, 0, 100, 0)], [a], send, () => 720);
    c.update([row(101, 1, 120, 10)], [a], send, () => 720);
    for (let i = 1; i <= 3; i++)
      c.update([row(101, i + 1, 120 + i * 30, 10)], [a], send, () => 720);
    expect(out.at(-1)?.congested).toBe(true);
    c.update([row(101, 5, 240, 10)], [a], send, () => 720);
    expect(out.at(-1)?.congested).toBe(false);
  });
  it("retains qualified congestion through stale, missing, malformed and reset counters", () => {
    const c = new ViewerLayerController(),
      out: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => out.push(q);
    c.update([row(101, 10, 100, 0)], [a], send, () => 180);
    c.update([row(101, 20, 120, 10)], [a], send, () => 180);
    for (const rows of [
      [row(101, 15, 999, 10)],
      [],
      [row(101, 30, NaN, 10)],
      [row(101, 40, 1, 0)],
    ]) {
      c.update(rows, [a], send, () => 1080);
      expect(out.at(-1)).toMatchObject({ h: 1080, congested: true });
    }
  });
  it("does not carry loss evidence into a replacement subscription generation", () => {
    const c = new ViewerLayerController(),
      out: MediaRequests["q"][] = [],
      send = (q: MediaRequests["q"]) => out.push(q);
    c.update([row(101, 0, 100, 0)], [a], send, () => 180);
    c.update([row(101, 1, 120, 10)], [a], send, () => 180);
    c.update([], [{ ...a, generation: "replacement" }], send, () => 1080);
    expect(out.at(-1)).toMatchObject({
      generation: "replacement",
      congested: false,
    });
  });
});
