import { afterEach, describe, expect, it, vi } from "vitest";

import { Gateway, type SocketLike } from "./client.ts";
import type { ChatEvent } from "./protocol.ts";

class FakeSocket implements SocketLike {
  sent: string[] = [];
  private listeners = new Map<string, Set<(event: { data?: string }) => void>>();

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.emit("close");
  }

  addEventListener(
    type: "open" | "message" | "close" | "error",
    handler: (event: { data?: string }) => void,
  ): void {
    const set = this.listeners.get(type) ?? new Set();
    set.add(handler);
    this.listeners.set(type, set);
  }

  removeEventListener(
    type: "open" | "message" | "close" | "error",
    handler: (event: { data?: string }) => void,
  ): void {
    this.listeners.get(type)?.delete(handler);
  }

  emit(type: "open" | "message" | "close" | "error", data?: string): void {
    for (const handler of this.listeners.get(type) ?? []) {
      handler({ data });
    }
  }
}

describe("gateway client", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("resubscribes with last seq after a drop and ignores duplicate events", async () => {
    const sockets: FakeSocket[] = [];
    const gateway = new Gateway({
      url: "ws://test/ws",
      retryMs: 10,
      maxRetryMs: 10,
      heartbeatMs: 10_000,
      deadMs: 20_000,
      open: () => {
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
    });

    const seen: ChatEvent[] = [];
    gateway.onEvent((event) => seen.push(event));
    gateway.setTopics([{ s: "srv", c: "ch" }]);
    gateway.start();
    sockets[0]?.emit("open");

    expect(sockets[0]?.sent).toContain('{"op":"s","s":"srv","c":"ch"}');

    sockets[0]?.emit(
      "message",
      '{"op":"ok","s":"srv","c":"ch","n":2}',
    );
    sockets[0]?.emit(
      "message",
      '{"op":"e","t":"c","s":"srv","c":"ch","n":3,"i":"m3"}',
    );
    sockets[0]?.emit(
      "message",
      '{"op":"e","t":"c","s":"srv","c":"ch","n":3,"i":"m3"}',
    );
    expect(seen).toHaveLength(1);

    sockets[0]?.emit("close");
    await vi.waitFor(() => {
      expect(sockets.length).toBe(2);
    });
    sockets[1]?.emit("open");

    expect(sockets[1]?.sent).toContain('{"op":"s","s":"srv","c":"ch","n":3}');

    sockets[1]?.emit(
      "message",
      '{"op":"e","t":"e","s":"srv","c":"ch","n":3,"i":"m3"}',
    );
    sockets[1]?.emit(
      "message",
      '{"op":"e","t":"c","s":"srv","c":"ch","n":4,"i":"m4"}',
    );
    expect(seen.map((event) => event.n)).toEqual([3, 4]);

    gateway.stop();
  });

  it("replies to heartbeats and reconnects after silence", async () => {
    vi.useFakeTimers();
    let now = 0;
    const sockets: FakeSocket[] = [];
    const gateway = new Gateway({
      url: "ws://test/ws",
      retryMs: 5,
      maxRetryMs: 5,
      heartbeatMs: 20,
      deadMs: 40,
      now: () => now,
      open: () => {
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
    });

    gateway.start();
    sockets[0]?.emit("open");
    sockets[0]?.emit("message", '{"op":"h"}');
    expect(sockets[0]?.sent).toContain('{"op":"h"}');

    now = 50;
    await vi.advanceTimersByTimeAsync(25);
    expect(sockets.length).toBe(2);

    gateway.stop();
  });
});
