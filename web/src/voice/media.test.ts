import { afterEach, describe, expect, it, vi } from "vitest";
import {
  decodeMediaFrame,
  MEDIA_VERSION,
  MediaError,
  type MediaClientFrame,
  type MediaServerFrame,
  type MediaSocket,
} from "./media.ts";
import { MediaPeer, MEDIA_REQUEST_DEADLINE_MS } from "./mediaPeer.ts";
const GENERATION = "11111111-1111-4111-8111-111111111111";
const OWNER = "22222222-2222-4222-8222-222222222222";
const EPOCH = "33333333-3333-4333-8333-333333333333";
function socketFixture() {
  const sent: MediaClientFrame[] = [];
  let frame: ((f: MediaServerFrame) => void) | null = null;
  let closed: (() => void) | null = null;
  const socket: MediaSocket = {
    send: (f) => {
      sent.push(f);
    },
    close: () => {
      closed?.();
    },
    onFrame: (handler) => {
      frame = handler;
      return () => {
        frame = null;
      };
    },
    onClose: (handler) => {
      closed = handler;
      return () => {
        closed = null;
      };
    },
  };
  const peer = new MediaPeer();
  peer.bind(socket, () => {});
  return {
    peer,
    sent,
    emit: (f: MediaServerFrame) => frame?.(f),
    disconnect: () => closed?.(),
  };
}
const joined = {
  c: OWNER,
  u: OWNER,
  v: MEDIA_VERSION,
  generation: GENERATION,
  routerRtpCapabilities: { codecs: [] },
};
async function accept(f: ReturnType<typeof socketFixture>) {
  const pending = f.peer.request("j", { tk: "abcdefghjkmn", v: MEDIA_VERSION });
  f.emit({ op: "result", id: f.sent.at(-1)!.id, data: joined });
  await pending;
}
afterEach(() => vi.useRealTimers());
describe("media v4 envelopes", () => {
  it("requires request identities and rejects all legacy SDP messages", () => {
    for (const raw of [
      null,
      [],
      { op: "result", id: 0, data: {} },
      { op: "result", id: 1.5, data: {} },
      { op: "result", id: 0x100000000, data: {} },
      { op: "o", sdp: "v=0" },
      { op: "a", sdp: "v=0" },
      { op: "i", ice: "candidate:x" },
      { op: "ok", v: 2 },
    ])
      expect(decodeMediaFrame(raw)).toBeNull();
    expect(decodeMediaFrame({ op: "result", id: 7, data: {} })).toEqual({
      op: "result",
      id: 7,
      data: {},
    });
  });
  it("binds explicit source kind, owner, epoch, consumer generation and audio parent", () => {
    const consumer = {
      op: "consumer",
      consumerId: "consumer-a",
      producerId: "producer-a",
      owner: OWNER,
      epoch: EPOCH,
      generation: GENERATION,
      k: "sa",
      kind: "audio",
      parent: "producer-screen",
      rtpParameters: { codecs: [] },
      paused: false,
    };
    expect(decodeMediaFrame(consumer)).toEqual(consumer);
    for (const change of [
      { parent: undefined },
      { owner: "track:owner" },
      { epoch: 1 },
      { generation: "old" },
      { k: "unknown" },
      { kind: "video" },
      { paused: 1 },
      { rtpParameters: [] },
    ])
      expect(decodeMediaFrame({ ...consumer, ...change })).toBeNull();
  });
  it("rejects malformed state and layer events", () => {
    const base = { consumerId: "c", generation: GENERATION };
    expect(
      decodeMediaFrame({ op: "consumerState", ...base, paused: false }),
    ).toBeTruthy();
    expect(
      decodeMediaFrame({ op: "consumerState", ...base, paused: "false" }),
    ).toBeNull();
    expect(
      decodeMediaFrame({
        op: "layers",
        ...base,
        spatialLayer: null,
        temporalLayer: null,
      }),
    ).toBeTruthy();
    expect(
      decodeMediaFrame({
        op: "layers",
        ...base,
        spatialLayer: -1,
        temporalLayer: 0,
      }),
    ).toBeNull();
    expect(
      decodeMediaFrame({
        op: "layers",
        ...base,
        spatialLayer: 0,
        temporalLayer: 1.1,
      }),
    ).toBeNull();
  });
});
describe("owned correlated media RPC", () => {
  it("sends one monotonic owned Leave before local socket teardown without waiting for a reply", async () => {
    const f = socketFixture();
    await accept(f);
    const pending = f.peer
      .request("w", { u: OWNER, k: "s", on: true })
      .catch((e: unknown) => e);
    f.peer.close();
    expect(f.sent.at(-1)).toEqual({ op: "l", id: 3 });
    expect(await pending).toMatchObject({ code: "connection_closed" });
    expect(f.peer.isOpen()).toBe(false);
    expect(f.peer.accepted).toBe(false);
    f.peer.close();
    expect(f.sent.filter((frame) => frame.op === "l")).toHaveLength(1);
  });
  it("does not claim Leave authority before Join or after the socket is gone", async () => {
    const initial = socketFixture();
    initial.peer.close();
    expect(initial.sent).toHaveLength(0);
    const joinedPeer = socketFixture();
    await accept(joinedPeer);
    joinedPeer.disconnect();
    joinedPeer.peer.close();
    expect(joinedPeer.sent.filter((frame) => frame.op === "l")).toHaveLength(0);
  });
  it("gates operations on successful versioned Join", async () => {
    const f = socketFixture();
    await expect(
      f.peer.request("transport", { direction: "send" }),
    ).rejects.toMatchObject({ code: "join_required" });
    await accept(f);
    expect(f.peer.accepted).toBe(true);
    const request = f.peer.request("transport", { direction: "recv" });
    expect(f.sent.at(-1)).toMatchObject({
      op: "transport",
      direction: "recv",
      id: 2,
    });
    f.emit({ op: "result", id: 2, data: { id: "recv" } });
    await expect(request).resolves.toEqual({ id: "recv" });
    f.peer.close();
  });
  it("rejects old protocol with a specific update error", async () => {
    const f = socketFixture();
    const promise = f.peer.request("j", {
      tk: "abcdefghjkmn",
      v: MEDIA_VERSION,
    });
    f.emit({ op: "result", id: 1, data: { ...joined, v: 2 } });
    await expect(promise).rejects.toMatchObject({ code: "update_required" });
    expect(f.peer.accepted).toBe(false);
    f.peer.close();
  });
  it("correlates out-of-order results and ignores duplicate/unknown response IDs", async () => {
    const f = socketFixture();
    await accept(f);
    const one = f.peer.request("w", { u: OWNER, k: "s", on: true });
    const two = f.peer.request("w", { u: OWNER, k: "l", on: true });
    f.emit({ op: "result", id: 3, data: { second: true } });
    f.emit({ op: "result", id: 90, data: {} });
    f.emit({ op: "result", id: 2, data: { first: true } });
    await expect(one).resolves.toEqual({ first: true });
    await expect(two).resolves.toEqual({ second: true });
    f.peer.close();
  });
  it("keeps the Live claim on a correlated denial", async () => {
    const f = socketFixture();
    await accept(f);
    const request = f.peer.request("produce", {
      k: "l",
      epoch: EPOCH,
      lc: EPOCH,
      rtp: { codecs: [] },
    });
    f.emit({ op: "err", id: 2, e: "forbidden", lc: EPOCH });
    await expect(request).rejects.toEqual(new MediaError("forbidden", EPOCH));
    f.peer.close();
  });
  it("bounds outstanding requests and cancels every promise on socket close", async () => {
    const f = socketFixture();
    await accept(f);
    const promises = Array.from({ length: 64 }, () =>
      f.peer
        .request("w", { u: OWNER, k: "s", on: true })
        .catch((e: unknown) => e),
    );
    await expect(
      f.peer.request("w", { u: OWNER, k: "s", on: true }),
    ).rejects.toMatchObject({ code: "request_overflow" });
    f.disconnect();
    const errors = await Promise.all(promises);
    expect(errors).toHaveLength(64);
    expect(
      errors.every(
        (e) => e instanceof MediaError && e.code === "connection_closed",
      ),
    ).toBe(true);
    expect(f.peer.isOpen()).toBe(false);
  });
  it("expires unresolved operations and rejects a late result even if its timer has not run", async () => {
    vi.useFakeTimers();
    const f = socketFixture();
    await accept(f);
    const request = f.peer.request("w", { u: OWNER, k: "s", on: true });
    const rejected = expect(request).rejects.toMatchObject({
      code: "request_timeout",
    });
    vi.setSystemTime(Date.now() + MEDIA_REQUEST_DEADLINE_MS + 1);
    f.emit({ op: "result", id: 2, data: {} });
    await rejected;
    f.peer.close();
  });
  it("uses the local absolute remaining deadline without serializing it or resetting ordinary RPC budgets", async () => {
    vi.useFakeTimers();
    const f = socketFixture();
    await accept(f);
    const localDeadline = Date.now() + 75;
    const timed = f.peer.request(
      "produce",
      { k: "l", epoch: EPOCH, lc: EPOCH, rtp: { codecs: [] } },
      localDeadline,
    );
    const rejected = expect(timed).rejects.toMatchObject({
      code: "request_timeout",
    });
    expect(Object.hasOwn(f.sent.at(-1)!, "deadlineEpochMs")).toBe(false);
    const ordinary = f.peer.request("w", { u: OWNER, k: "s", on: true });
    let ordinarySettled = false;
    void ordinary.then(
      () => {
        ordinarySettled = true;
      },
      () => {
        ordinarySettled = true;
      },
    );
    await vi.advanceTimersByTimeAsync(75);
    await rejected;
    expect(ordinarySettled).toBe(false);
    f.emit({ op: "result", id: 3, data: {} });
    await expect(ordinary).resolves.toEqual({});
    f.peer.close();
  });
  it("does not issue expired local requests and does not consume their monotonic wire IDs", async () => {
    vi.useFakeTimers();
    const f = socketFixture();
    await accept(f);
    const before = f.sent.length;
    await expect(
      f.peer.request("w", { u: OWNER, k: "l", on: true }, Date.now()),
    ).rejects.toMatchObject({ code: "request_timeout" });
    expect(f.sent).toHaveLength(before);
    const next = f.peer.request(
      "w",
      { u: OWNER, k: "l", on: true },
      Date.now() + 50,
    );
    expect(f.sent.at(-1)?.id).toBe(2);
    f.emit({ op: "result", id: 2, data: {} });
    await expect(next).resolves.toEqual({});
    f.peer.close();
  });
  it("rejects an RPC continuation that resumes after the absolute budget despite an earlier valid response", async () => {
    vi.useFakeTimers();
    const f = socketFixture();
    await accept(f);
    const request = f.peer.request(
      "w",
      { u: OWNER, k: "l", on: true },
      Date.now() + 50,
    );
    f.emit({ op: "result", id: 2, data: {} });
    vi.setSystemTime(Date.now() + 51);
    await expect(request).rejects.toMatchObject({ code: "request_timeout" });
    f.peer.close();
  });
  it("does not resurrect a Join resolved immediately before local close", async () => {
    const f = socketFixture();
    const promise = f.peer.request("j", {
      tk: "abcdefghjkmn",
      v: MEDIA_VERSION,
    });
    f.emit({ op: "result", id: 1, data: joined });
    f.peer.close();
    await expect(promise).rejects.toMatchObject({ code: "connection_closed" });
    expect(f.peer.accepted).toBe(false);
  });
  it("closes an exhausted request generation instead of reusing an earlier id", async () => {
    const f = socketFixture();
    await accept(f);
    // Advance to the last u32 identity without issuing four billion requests.
    (f.peer as unknown as { nextId: number }).nextId = 0xffffffff;
    const last = f.peer
      .request("w", { u: OWNER, k: "s", on: true })
      .catch((e: unknown) => e);
    expect(f.sent.at(-1)?.id).toBe(0xffffffff);
    await expect(
      f.peer.request("w", { u: OWNER, k: "l", on: true }),
    ).rejects.toMatchObject({ code: "request_id_exhausted" });
    expect(await last).toMatchObject({ code: "connection_closed" });
    expect(f.peer.isOpen()).toBe(false);
    expect(f.sent).toHaveLength(2);
  });
});
