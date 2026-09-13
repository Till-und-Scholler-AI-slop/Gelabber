import { describe, expect, it } from "vitest";

import {
  decode,
  encode,
  nextCursor,
  resumeFrame,
  topicKey,
} from "./protocol.ts";

describe("ws protocol", () => {
  it("keeps subscribe frames compact", () => {
    expect(encode({ op: "h" })).toBe('{"op":"h"}');
    expect(JSON.parse(encode({ op: "s", s: "srv", c: "ch", n: 4 }))).toEqual({
      op: "s",
      s: "srv",
      c: "ch",
      n: 4,
    });
  });

  it("decodes events and rejects unknown ops (signaling stays off this stream)", () => {
    const event = decode(
      '{"op":"e","t":"c","s":"srv","c":"ch","n":3,"i":"m1","d":{"b":"hi"}}',
    );
    expect(event).toEqual({
      op: "e",
      t: "c",
      s: "srv",
      c: "ch",
      n: 3,
      i: "m1",
      d: { b: "hi" },
    });
    expect(decode('{"op":"sig","t":"offer"}')).toBeNull();
    expect(decode("not-json")).toBeNull();
  });

  it("resumes with last seq and drops duplicates", () => {
    expect(resumeFrame({ s: "srv", c: "ch" }, 9)).toEqual({
      op: "s",
      s: "srv",
      c: "ch",
      n: 9,
    });
    expect(resumeFrame({ s: "srv" }, undefined)).toEqual({
      op: "s",
      s: "srv",
    });
    expect(topicKey({ s: "srv", c: "ch" })).toBe("c:ch");
    expect(nextCursor(5, 5)).toEqual({ accept: false, cursor: 5 });
    expect(nextCursor(5, 6)).toEqual({ accept: true, cursor: 6 });
    expect(nextCursor(undefined, 1)).toEqual({ accept: true, cursor: 1 });
  });
});
