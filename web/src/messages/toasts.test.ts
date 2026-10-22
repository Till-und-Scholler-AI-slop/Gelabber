import { afterEach, describe, expect, it, vi } from "vitest";

import { TOAST_HOLD_MS } from "./notify.ts";
import {
  RELEASE_MIN_MS,
  resetMessageToastsForTests,
  useMessageToasts,
} from "./toasts.ts";

const sample = {
  channelId: "c1",
  serverId: "srv",
  dm: false,
  channelLabel: "#allgemein",
  author: "Ada",
  preview: "hallo",
};

describe("message toast store", () => {
  afterEach(() => {
    vi.useRealTimers();
    resetMessageToastsForTests();
  });

  it("stacks a burst on the same channel and keeps a second channel separate", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    useMessageToasts.getState().push(sample);
    vi.setSystemTime(1_400);
    useMessageToasts.getState().push({ ...sample, preview: "nochmal" });
    useMessageToasts.getState().push({
      ...sample,
      channelId: "c2",
      channelLabel: "#voice",
      preview: "andere",
    });
    const toasts = useMessageToasts.getState().toasts;
    expect(toasts).toHaveLength(2);
    expect(toasts[0]?.count).toBe(2);
    expect(toasts[0]?.preview).toBe("nochmal");
    expect(toasts[1]?.channelId).toBe("c2");
  });

  it("pauses while pointed at or focused, and resumes with time to read", () => {
    vi.useFakeTimers();
    useMessageToasts.getState().push(sample);
    const id = useMessageToasts.getState().toasts[0]!.id;
    vi.advanceTimersByTime(TOAST_HOLD_MS - 500);
    useMessageToasts.getState().hold(id, "hover");
    useMessageToasts.getState().hold(id, "focus");
    vi.advanceTimersByTime(TOAST_HOLD_MS * 3);
    expect(useMessageToasts.getState().toasts).toHaveLength(1);
    useMessageToasts.getState().release(id, "hover");
    vi.advanceTimersByTime(TOAST_HOLD_MS);
    expect(useMessageToasts.getState().toasts).toHaveLength(1);
    useMessageToasts.getState().release(id, "focus");
    vi.advanceTimersByTime(RELEASE_MIN_MS - 1);
    expect(useMessageToasts.getState().toasts).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(useMessageToasts.getState().toasts).toHaveLength(0);
  });

  it("counts a held toast's news from the full time once let go", () => {
    vi.useFakeTimers();
    useMessageToasts.getState().push(sample);
    const id = useMessageToasts.getState().toasts[0]!.id;
    useMessageToasts.getState().hold(id, "hover");
    useMessageToasts.getState().push({ ...sample, preview: "noch was" });
    expect(useMessageToasts.getState().toasts[0]?.count).toBe(2);
    useMessageToasts.getState().release(id, "hover");
    vi.advanceTimersByTime(TOAST_HOLD_MS - 1);
    expect(useMessageToasts.getState().toasts).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(useMessageToasts.getState().toasts).toHaveLength(0);
  });
});
