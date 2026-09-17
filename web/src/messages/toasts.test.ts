import { afterEach, describe, expect, it, vi } from "vitest";

import { resetMessageToastsForTests, useMessageToasts } from "./toasts.ts";

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
});
