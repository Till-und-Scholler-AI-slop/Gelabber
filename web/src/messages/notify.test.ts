import { describe, expect, it } from "vitest";

import {
  isDmTopic,
  createNotificationDedupe,
  messageNotificationDecision,
  previewText,
  shouldToastMessage,
  stackOnto,
} from "./notify.ts";

describe("message toast rules", () => {
  it("delivers desktop messages independently, including the hidden open chat", () => {
    const base = {
      toastEnabled: false,
      desktopEnabled: true,
      hidden: true,
      type: "c" as const,
      own: false,
      channelId: "c1",
      viewingChannelId: "c1",
    };
    expect(messageNotificationDecision(base)).toEqual({
      toast: false,
      desktop: true,
    });
    expect(
      messageNotificationDecision({ ...base, hidden: false }).desktop,
    ).toBe(false);
    expect(messageNotificationDecision({ ...base, own: true }).desktop).toBe(
      false,
    );
    expect(messageNotificationDecision({ ...base, type: "e" }).desktop).toBe(
      false,
    );
    expect(
      messageNotificationDecision({ ...base, desktopEnabled: false }).desktop,
    ).toBe(false);
    expect(
      messageNotificationDecision({
        ...base,
        toastEnabled: true,
        viewingChannelId: "c2",
        desktopEnabled: false,
      }),
    ).toEqual({ toast: true, desktop: false });
  });

  it("does not replay delivery and bounds session deduplication", () => {
    const first = createNotificationDedupe(2);
    expect(first("c", "m1")).toBe(true);
    expect(first("c", "m1")).toBe(false);
    expect(first("other", "m1")).toBe(true);
    expect(first("c", "m2")).toBe(true);
    expect(first("c", "m1")).toBe(true);
  });
  it("skips own messages, edits, the open channel, and a disabled setting", () => {
    const base = {
      enabled: true,
      type: "c" as const,
      own: false,
      channelId: "c1",
      viewingChannelId: "c2",
    };
    expect(shouldToastMessage(base)).toBe(true);
    expect(shouldToastMessage({ ...base, own: true })).toBe(false);
    expect(shouldToastMessage({ ...base, type: "e" })).toBe(false);
    expect(shouldToastMessage({ ...base, viewingChannelId: "c1" })).toBe(false);
    expect(shouldToastMessage({ ...base, enabled: false })).toBe(false);
    expect(shouldToastMessage({ ...base, viewingChannelId: undefined })).toBe(
      true,
    );
  });

  it("treats protocol s === c as a DM and truncates previews", () => {
    expect(isDmTopic("dm-1", "dm-1")).toBe(true);
    expect(isDmTopic("srv", "c1")).toBe(false);
    expect(previewText("  hallo  welt  ", false)).toBe("hallo welt");
    expect(previewText("", true)).toBe("Datei");
    expect(previewText("x".repeat(200), false).endsWith("…")).toBe(true);
    expect(previewText("x".repeat(200), false).length).toBeLessThanOrEqual(140);
  });

  it("stacks bursts on the same channel and not after the window", () => {
    const open = [{ id: 1, channelId: "c1", at: 1_000, count: 1 }];
    expect(stackOnto(open, "c1", 1_500)?.id).toBe(1);
    expect(stackOnto(open, "c1", 4_000)).toBeNull();
    expect(stackOnto(open, "c2", 1_100)).toBeNull();
  });
});
