import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearMessageJump,
  JUMP_TTL_MS,
  peekMessageJump,
  requestMessageJump,
} from "./messageJump.ts";

afterEach(() => {
  clearMessageJump("c1");
  vi.useRealTimers();
});

describe("message jump", () => {
  it("opens only the channel it was asked for, until cleared", () => {
    requestMessageJump({ channelId: "c1", messageId: "m1" });
    expect(peekMessageJump("c2")).toBeNull();
    expect(peekMessageJump("c1")).toBe("m1");
    clearMessageJump("c1");
    expect(peekMessageJump("c1")).toBeNull();
  });

  it("is dropped when no page took it up in time", () => {
    vi.useFakeTimers();
    requestMessageJump({ channelId: "c1", messageId: "m1" });
    vi.advanceTimersByTime(JUMP_TTL_MS + 1);
    expect(peekMessageJump("c1")).toBeNull();
  });
});
