import { afterEach, describe, expect, it, vi } from "vitest";

import { Gateway, type SocketLike } from "./client.ts";
import type { ChatEvent } from "./protocol.ts";

class FakeSocket implements SocketLike {
  sent: string[] = [];
  private listeners = new Map<
    string,
    Set<(event: { data?: string }) => void>
  >();

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

    sockets[0]?.emit("message", '{"op":"ok","s":"srv","c":"ch","n":2}');
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

  it("routes signaling off the chat event stream", () => {
    const sockets: FakeSocket[] = [];
    const gateway = new Gateway({
      url: "ws://test/ws",
      open: () => {
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
    });

    const chat: ChatEvent[] = [];
    const sigs: { t: string }[] = [];
    gateway.onEvent((event) => chat.push(event));
    gateway.onSig((event) => sigs.push(event));
    gateway.start();
    sockets[0]?.emit("open");
    gateway.send({ op: "sig", t: "j", s: "srv", c: "voice" });
    expect(sockets[0]?.sent).toContain(
      '{"op":"sig","t":"j","s":"srv","c":"voice"}',
    );

    sockets[0]?.emit(
      "message",
      '{"op":"sig","t":"j","s":"srv","c":"voice","u":"u1"}',
    );
    sockets[0]?.emit("message", '{"op":"e","t":"c","s":"srv","c":"ch","n":1}');
    expect(sigs).toEqual([
      { op: "sig", t: "j", s: "srv", c: "voice", u: "u1" },
    ]);
    expect(chat).toHaveLength(1);

    gateway.stop();
  });

  it("forwards presence and typing without touching chat cursors", () => {
    const sockets: FakeSocket[] = [];
    const gateway = new Gateway({
      url: "ws://test/ws",
      open: () => {
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const presence: unknown[] = [];
    const typing: unknown[] = [];
    const events: ChatEvent[] = [];
    gateway.onPresence((frame) => presence.push(frame));
    gateway.onTyping((frame) => typing.push(frame));
    gateway.onEvent((event) => events.push(event));
    gateway.setTopics([{ s: "srv", c: "ch" }]);
    gateway.start();
    sockets[0]?.emit("open");
    sockets[0]?.emit("message", '{"op":"ok","s":"srv","c":"ch","n":2}');
    sockets[0]?.emit("message", '{"op":"p","s":"srv","u":"u1","st":"o"}');
    sockets[0]?.emit(
      "message",
      '{"op":"y","s":"srv","c":"ch","u":"u1","on":true}',
    );
    expect(presence).toEqual([{ op: "p", s: "srv", u: "u1", st: "o" }]);
    expect(typing).toEqual([{ op: "y", s: "srv", c: "ch", u: "u1", on: true }]);
    expect(events).toHaveLength(0);
    expect(gateway.cursorsSnapshot.get("c:ch")).toBe(2);

    gateway.sendPresence("i");
    gateway.sendTyping("srv", "ch", false);
    expect(sockets[0]?.sent).toContain('{"op":"p","st":"i"}');
    expect(sockets[0]?.sent).toContain(
      '{"op":"y","s":"srv","c":"ch","on":false}',
    );
    gateway.stop();
  });

  it("resetSession forgets topics and resume cursors", () => {
    const sockets: FakeSocket[] = [];
    const gateway = new Gateway({
      url: "ws://test/ws",
      open: () => {
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
    });
    const seen: ChatEvent[] = [];
    gateway.onEvent((event) => {
      seen.push(event);
      gateway.resetSession();
    });
    gateway.setTopics([{ s: "srv", c: "ch" }]);
    gateway.start();
    sockets[0]?.emit("open");
    sockets[0]?.emit("message", '{"op":"ok","s":"srv","c":"ch","n":4}');
    expect(gateway.cursorsSnapshot.get("c:ch")).toBe(4);

    sockets[0]?.emit(
      "message",
      '{"op":"e","t":"c","s":"srv","c":"ch","n":5,"i":"m5"}',
    );
    expect(seen).toHaveLength(1);
    expect(gateway.cursorsSnapshot.size).toBe(0);

    gateway.setTopics([{ s: "srv", c: "ch" }]);
    gateway.start();
    sockets.at(-1)?.emit("open");
    const latest = sockets.at(-1);
    expect(latest?.sent).toContain('{"op":"s","s":"srv","c":"ch"}');
    expect(latest?.sent.some((frame) => frame.includes('"n"'))).toBe(false);
    gateway.stop();
  });
});
