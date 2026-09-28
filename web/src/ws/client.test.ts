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

  it("keeps gap heads pending and rejects acknowledgements from an older gap or session", () => {
    const sockets: FakeSocket[] = [];
    const gateway = new Gateway({
      open: () => {
        const socket = new FakeSocket();
        sockets.push(socket);
        return socket;
      },
    });
    gateway.start();
    sockets[0]!.emit("open");
    const frame = (data: unknown) =>
      sockets.at(-1)!.emit("message", JSON.stringify(data));
    frame({ op: "ok", s: "srv", c: "ch", n: 2 });
    frame({ op: "gap", s: "srv", c: "ch" });
    frame({ op: "ok", s: "srv", c: "ch", n: 9 });
    const old = gateway.gapRecoveries()[0]!;
    expect(gateway.cursorsSnapshot.get("c:ch")).toBe(2);
    frame({ op: "gap", s: "srv", c: "ch" });
    frame({ op: "ok", s: "srv", c: "ch", n: 12 });
    const current = gateway.gapRecoveries()[0]!;
    gateway.completeGap(old);
    expect(gateway.gapRecoveries()).toEqual([current]);
    expect(gateway.cursorsSnapshot.get("c:ch")).toBe(2);
    gateway.completeGap(current);
    expect(gateway.cursorsSnapshot.get("c:ch")).toBe(12);
    gateway.resetSession();
    gateway.start();
    sockets.at(-1)!.emit("open");
    frame({ op: "gap", s: "srv", c: "ch" });
    frame({ op: "ok", s: "srv", c: "ch", n: 1 });
    gateway.completeGap(current);
    expect(gateway.cursorsSnapshot.get("c:ch")).toBeUndefined();
    expect(gateway.gapRecoveries()).toHaveLength(1);
    gateway.completeGap(gateway.gapRecoveries()[0]!);
    expect(gateway.cursorsSnapshot.get("c:ch")).toBe(1);
    gateway.stop();
  });

  it("a late close of the previous socket does not drop the next one", async () => {
    class DeferredCloseSocket extends FakeSocket {
      override close(): void {
        // The browser fires close on a later turn. The test emits it.
      }
    }

    const sockets: DeferredCloseSocket[] = [];
    const gateway = new Gateway({
      url: "ws://test/ws",
      heartbeatMs: 60_000,
      open: () => {
        const socket = new DeferredCloseSocket();
        sockets.push(socket);
        return socket;
      },
    });

    gateway.start();
    sockets[0]?.emit("open");
    gateway.resetSession();
    gateway.start();
    const next = sockets[1];
    next?.emit("open");
    expect(sockets).toHaveLength(2);

    sockets[0]?.emit("close");
    sockets[0]?.emit("message", '{"op":"h"}');
    sockets[0]?.emit("open");
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(sockets).toHaveLength(2);
    expect(next?.sent.some((frame) => frame.includes('"op":"h"'))).toBe(false);
    gateway.send({ op: "h" });
    expect(next?.sent).toContain('{"op":"h"}');

    gateway.stop();
    sockets[1]?.emit("close");
    gateway.send({ op: "h" });
    expect(sockets).toHaveLength(2);
  });
});

describe("epoch-aware transport", () => {
  function setup() {
    const socket = new FakeSocket();
    const gateway = new Gateway({ open: () => socket });
    const events: ChatEvent[] = [];
    gateway.onEvent((event) => events.push(event));
    gateway.setTopics([{ s: "s", c: "c" }]);
    gateway.start();
    socket.emit("open");
    const frame = (body: unknown) =>
      socket.emit("message", JSON.stringify(body));
    return { gateway, socket, events, frame };
  }
  it("accepts smaller new-epoch events and resumes with the new pair only after REST", () => {
    const { gateway, socket, events, frame } = setup();
    try {
      frame({ op: "ok", s: "s", c: "c", ep: "old", n: 500 });
      frame({ op: "gap", s: "s", c: "c", ep: "new" });
      frame({ op: "ok", s: "s", c: "c", ep: "new", n: 0 });
      frame({ op: "e", s: "s", c: "c", ep: "new", n: 1, t: "c", i: "m", r: 9 });
      frame({ op: "e", s: "s", c: "c", ep: "new", n: 1, t: "c", i: "m", r: 9 });
      expect(events.map((e) => e.n)).toEqual([1]);
      expect(gateway.topicCursorsSnapshot.get("c:c")).toEqual({
        ep: "old",
        n: 500,
      });
      gateway.completeGap(gateway.gapRecoveries()[0]!);
      expect(gateway.topicCursorsSnapshot.get("c:c")).toEqual({
        ep: "new",
        n: 1,
      });
      socket.sent.length = 0;
      gateway.setTopics([]);
      gateway.setTopics([{ s: "s", c: "c" }]);
      expect(socket.sent.map((s) => JSON.parse(s))).toContainEqual({
        op: "s",
        s: "s",
        c: "c",
        ep: "new",
        n: 1,
      });
    } finally {
      gateway.stop();
    }
  });
  it("does not finalize a fast REST read before the authoritative ok head arrives", () => {
    const { gateway, frame } = setup();
    try {
      frame({ op: "ok", s: "s", c: "c", ep: "epoch", n: 100 });
      frame({ op: "gap", s: "s", c: "c", ep: "epoch" });
      gateway.completeGap(gateway.gapRecoveries()[0]!);
      expect(gateway.topicCursorsSnapshot.get("c:c")).toEqual({
        ep: "epoch",
        n: 100,
      });
      expect(gateway.gapRecoveries()).toEqual([]); // Already reconciled, no REST loop while waiting for ok.
      frame({ op: "ok", s: "s", c: "c", ep: "epoch", n: 0 });
      expect(gateway.topicCursorsSnapshot.get("c:c")).toEqual({
        ep: "epoch",
        n: 0,
      });
    } finally {
      gateway.stop();
    }
  });
  it("notices an epoch change on a live event even without a preceding gap", () => {
    const { gateway, events, frame } = setup();
    try {
      frame({ op: "ok", s: "s", c: "c", ep: "old", n: 100 });
      frame({ op: "e", s: "s", c: "c", ep: "new", n: 1, t: "e", i: "m", r: 3 });
      expect(events).toHaveLength(1);
      expect(gateway.gapRecoveries()).toHaveLength(1);
      gateway.completeGap(gateway.gapRecoveries()[0]!);
      expect(gateway.topicCursorsSnapshot.get("c:c")).toEqual({
        ep: "new",
        n: 1,
      });
    } finally {
      gateway.stop();
    }
  });
  it("resync resubscribes every topic and reaches sockets with no topics; private DM stays off chat", () => {
    const { gateway, socket, events, frame } = setup();
    let resyncs = 0;
    const discovered: string[] = [];
    gateway.onResync(() => resyncs++);
    gateway.onDm((id) => discovered.push(id));
    try {
      gateway.setTopics([{ s: "s" }, { s: "s", c: "c" }]);
      socket.sent.length = 0;
      frame({ op: "resync" });
      expect(
        socket.sent.map((s) => JSON.parse(s)).filter((s) => s.op === "s"),
      ).toHaveLength(2);
      gateway.setTopics([]);
      frame({ op: "resync" });
      frame({ op: "dm", c: "private" });
      expect(resyncs).toBe(2);
      expect(discovered).toEqual(["private"]);
      expect(events).toEqual([]);
    } finally {
      gateway.stop();
    }
  });
});

it("acknowledges an epoch gap followed only by live delivery after reconciliation", () => {
  const socket = new FakeSocket();
  const gateway = new Gateway({ open: () => socket });
  const events: ChatEvent[] = [];
  gateway.onEvent((e) => events.push(e));
  gateway.start();
  socket.emit("open");
  const frame = (body: unknown) => socket.emit("message", JSON.stringify(body));
  try {
    frame({ op: "ok", s: "s", c: "c", ep: "old", n: 100 });
    frame({ op: "gap", s: "s", c: "c", ep: "new" });
    frame({ op: "e", s: "s", c: "c", ep: "new", n: 1, t: "c", i: "m", r: 50 });
    expect(events).toHaveLength(1);
    expect(gateway.cursorsSnapshot.get("c:c")).toBe(100);
    gateway.completeGap(gateway.gapRecoveries()[0]!);
    expect(gateway.topicCursorsSnapshot.get("c:c")).toEqual({
      ep: "new",
      n: 1,
    });
  } finally {
    gateway.stop();
  }
});
