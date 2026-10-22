import { describe, expect, it } from "vitest";

import { LIVE_ARRIVALS_MAX, arrivals } from "./arrivals.ts";

const row = (id: string, author = "mia") => ({ id, author: { id: author } });
const rows = (...ids: string[]) => ids.map((id) => row(id));

describe("live arrivals", () => {
  it("are none on the first load", () => {
    expect(arrivals([], rows("1", "2"), "me").size).toBe(0);
  });

  it("are a new message below what was shown", () => {
    const found = arrivals(rows("1", "2"), rows("1", "2", "3"), "me");
    expect([...found]).toEqual([["3", "live"]]);
  });

  it("are not older pages coming in above", () => {
    expect(arrivals(rows("3", "4"), rows("1", "2", "3", "4"), "me").size).toBe(
      0,
    );
  });

  it("are not a jump that shows another stretch of the conversation", () => {
    expect(arrivals(rows("1", "2"), rows("7", "8"), "me").size).toBe(0);
  });

  it("are not a catch-up of many messages at once", () => {
    const many = Array.from({ length: LIVE_ARRIVALS_MAX + 1 }, (_, i) =>
      String(i + 3),
    );
    expect(arrivals(rows("1", "2"), rows("1", "2", ...many), "me").size).toBe(
      0,
    );
  });

  it("include one's own message on its way", () => {
    const found = arrivals(
      rows("1"),
      [row("1"), row("tmp:a", "me")],
      "me",
    );
    expect(found.get("tmp:a")).toBe("live");
  });

  it("call one's own saved message confirmed when it replaces its tmp row", () => {
    const found = arrivals(
      [row("1"), row("tmp:a", "me")],
      [row("1"), row("9", "me")],
      "me",
    );
    expect([...found]).toEqual([["9", "confirmed"]]);
  });

  it("call it confirmed too when the saved copy comes before its tmp row goes", () => {
    const found = arrivals(
      [row("1"), row("tmp:a", "me")],
      [row("1"), row("9", "me"), row("tmp:a", "me")],
      "me",
    );
    expect([...found]).toEqual([["9", "confirmed"]]);
  });

  it("keep a message from someone else live while one's own is confirmed", () => {
    const found = arrivals(
      [row("1"), row("tmp:a", "me")],
      [row("1"), row("8", "mia"), row("9", "me")],
      "me",
    );
    expect(found.get("8")).toBe("live");
    expect(found.get("9")).toBe("confirmed");
  });

  it("are nothing new when a row only changed", () => {
    expect(arrivals(rows("1", "2"), rows("1", "2"), "me").size).toBe(0);
  });
});
