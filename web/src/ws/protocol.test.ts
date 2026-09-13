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

  it("decodes presence and typing without treating them as chat events", () => {
    expect(decode('{"op":"p","s":"srv","u":"u1","st":"o"}')).toEqual({
      op: "p",
      s: "srv",
      u: "u1",
      st: "o",
    });
    expect(decode('{"op":"y","s":"srv","c":"ch","u":"u1","on":true}')).toEqual({
      op: "y",
      s: "srv",
      c: "ch",
      u: "u1",
      on: true,
    });
    expect(encode({ op: "p", st: "i" })).toBe('{"op":"p","st":"i"}');
    expect(encode({ op: "y", s: "srv", c: "ch", on: true })).toBe(
      '{"op":"y","s":"srv","c":"ch","on":true}',
    );
  });

  it("decodes chat and signaling on separate ops", () => {
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
    const sig = decode('{"op":"sig","t":"j","s":"srv","c":"voice","u":"u1"}');
    expect(sig).toEqual({
      op: "sig",
      t: "j",
      s: "srv",
      c: "voice",
      u: "u1",
    });
    expect(
      decode('{"op":"sig","t":"m","s":"srv","c":"voice","u":"u1","on":true}'),
    ).toEqual({
      op: "sig",
      t: "m",
      s: "srv",
      c: "voice",
      u: "u1",
      on: true,
    });
    expect(
      decode(
        '{"op":"sig","t":"r","s":"srv","snap":[{"u":"u1","c":"voice","m":true}]}',
      ),
    ).toEqual({
      op: "sig",
      t: "r",
      s: "srv",
      snap: [{ u: "u1", c: "voice", m: true }],
    });
    expect(decode('{"op":"lk","t":"offer"}')).toBeNull();
    expect(decode("not-json")).toBeNull();
  });

  it("keeps signaling frames small", () => {
    expect(encode({ op: "sig", t: "j", s: "srv", c: "voice" })).toBe(
      '{"op":"sig","t":"j","s":"srv","c":"voice"}',
    );
    expect(
      encode({
        op: "sig",
        t: "i",
        s: "srv",
        c: "voice",
        ice: "cand",
        mid: "0",
      }),
    ).toBe('{"op":"sig","t":"i","s":"srv","c":"voice","ice":"cand","mid":"0"}');
    expect(encode({ op: "sig", t: "p", s: "srv", c: "voice", k: "s" })).toBe(
      '{"op":"sig","t":"p","s":"srv","c":"voice","k":"s"}',
    );
    expect(encode({ op: "sig", t: "p", s: "srv", c: "voice", k: "l" })).toBe(
      '{"op":"sig","t":"p","s":"srv","c":"voice","k":"l"}',
    );
    expect(encode({ op: "sig", t: "m", s: "srv", c: "voice", on: true })).toBe(
      '{"op":"sig","t":"m","s":"srv","c":"voice","on":true}',
    );
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
