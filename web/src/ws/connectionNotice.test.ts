import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  connectionNotices,
  NOTICE_DELAY_MS,
  RESTORED_MS,
  type ConnectionNotice,
} from "./connectionNotice.ts";

describe("connection notice", () => {
  let shown: ConnectionNotice[];
  let notices: ReturnType<typeof connectionNotices>;
  beforeEach(() => {
    vi.useFakeTimers();
    shown = [];
    notices = connectionNotices((notice) => shown.push(notice));
  });
  afterEach(() => {
    notices.stop();
    vi.useRealTimers();
  });

  it("says nothing while connecting the first time or connected", () => {
    notices.update({ status: "connecting", online: true });
    notices.update({ status: "open", online: true });
    vi.advanceTimersByTime(NOTICE_DELAY_MS * 2);
    expect(shown).toEqual([]);
  });

  it("keeps a quick reconnect invisible", () => {
    notices.update({ status: "open", online: true });
    notices.update({ status: "reconnecting", online: true });
    vi.advanceTimersByTime(NOTICE_DELAY_MS - 1);
    notices.update({ status: "open", online: true });
    vi.advanceTimersByTime(NOTICE_DELAY_MS + RESTORED_MS);
    expect(shown).toEqual([]);
  });

  it("reports a longer drop and confirms the way back, then goes", () => {
    notices.update({ status: "open", online: true });
    notices.update({ status: "reconnecting", online: true });
    // Every retry reports again; the wait is not restarted by it.
    vi.advanceTimersByTime(NOTICE_DELAY_MS / 2);
    notices.update({ status: "reconnecting", online: true });
    vi.advanceTimersByTime(NOTICE_DELAY_MS / 2);
    expect(shown).toEqual(["reconnecting"]);
    notices.update({ status: "open", online: true });
    expect(shown).toEqual(["reconnecting", "restored"]);
    vi.advanceTimersByTime(RESTORED_MS);
    expect(shown).toEqual(["reconnecting", "restored", null]);
  });

  it("shows an offline device at once, and the reconnect after it", () => {
    notices.update({ status: "open", online: true });
    notices.update({ status: "open", online: false });
    expect(shown).toEqual(["offline"]);
    notices.update({ status: "reconnecting", online: false });
    notices.update({ status: "reconnecting", online: true });
    expect(shown).toEqual(["offline", "reconnecting"]);
    notices.update({ status: "open", online: true });
    expect(shown.at(-1)).toBe("restored");
  });

  it("confirms the way back when the socket outlived a short offline spell", () => {
    notices.update({ status: "open", online: false });
    notices.update({ status: "open", online: true });
    expect(shown).toEqual(["offline", "restored"]);
  });

  it("clears everything on sign-out", () => {
    notices.update({ status: "open", online: false });
    notices.update({ status: "idle", online: false });
    expect(shown).toEqual(["offline", null]);
    notices.update({ status: "reconnecting", online: true });
    notices.update({ status: "idle", online: true });
    vi.advanceTimersByTime(NOTICE_DELAY_MS);
    expect(shown).toEqual(["offline", null]);
  });
});
