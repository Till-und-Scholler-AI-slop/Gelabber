import { describe, expect, it, vi } from "vitest";

import type { NotificationSetting } from "../messages/notificationSettings.ts";
import type { ServerDetail } from "../servers/types.ts";
import type { SigEvent } from "../ws/protocol.ts";
import {
  LIVE_NOTICE_COOLDOWN_MS,
  LIVE_ROSTER_GRACE_MS,
  createLiveNotices,
  createLiveStarts,
  liveNoticeDecision,
  liveNotificationText,
  liveSoundWanted,
  liveText,
  type LiveDevice,
  type LiveNoticeDeps,
  type Shown,
} from "./liveNotices.ts";

const MINUTE = 60_000;

const server: ServerDetail = {
  id: "srv",
  name: "Runde",
  owner_id: "owner",
  created_at: "2026-10-01T00:00:00Z",
  role: "member",
  permissions: ["join_voice", "send_messages"],
  member_permissions: ["join_voice", "send_messages"],
  categories: [],
  channels: [
    {
      id: "lounge",
      server_id: "srv",
      category_id: null,
      name: "Lounge",
      kind: "voice",
      created_at: "2026-10-01T00:00:00Z",
    },
    {
      id: "bar",
      server_id: "srv",
      category_id: null,
      name: "Bar",
      kind: "voice",
      created_at: "2026-10-01T00:00:00Z",
    },
    {
      id: "chat",
      server_id: "srv",
      category_id: null,
      name: "allgemein",
      kind: "text",
      created_at: "2026-10-01T00:00:00Z",
    },
  ],
  members: [
    {
      user_id: "ada",
      name: "Ada",
      avatar_url: "/avatars/ada.png",
      joined_at: "2026-10-01T00:00:00Z",
      role: "member",
    },
  ],
};

const start = (
  u = "ada",
  c = "lounge",
  extra: Partial<SigEvent> = {},
): SigEvent => ({ op: "sig", t: "p", s: "srv", c, u, k: "s", ...extra });
const stop = (u = "ada", c = "lounge"): SigEvent => ({
  op: "sig",
  t: "u",
  s: "srv",
  c,
  u,
  k: "s",
});
const roster = (live: Array<[user: string, channel: string]>): SigEvent => ({
  op: "sig",
  t: "r",
  s: "srv",
  snap: live.map(([u, c]) => ({ u, c, s: true })),
});

const unset: NotificationSetting = {
  scope: "server",
  id: "srv",
  level: null,
  muted: false,
  muted_until: null,
};

/** A handler with a hidden window, toasts and desktop notifications on, and
 * every effect recorded. */
function harness(overrides: Partial<LiveNoticeDeps> = {}) {
  let now = 1_000_000;
  const device = {
    toasts: true,
    desktop: true,
    hidden: true,
    focused: false,
    viewingChannelId: undefined as string | undefined,
    mobile: false,
    deafened: false,
    hideContent: false,
  };
  const state = {
    here: null as string | null,
    setting: unset,
    shown: "page" as Shown,
    server: server as ServerDetail | undefined,
  };
  const toast = vi.fn<LiveNoticeDeps["toast"]>();
  const notify = vi.fn<LiveNoticeDeps["notify"]>(async () => state.shown);
  const sound = vi.fn<LiveNoticeDeps["sound"]>();
  const open = vi.fn<LiveNoticeDeps["open"]>();
  const handle = createLiveNotices({
    me: "me",
    now: () => now,
    serverNow: () => now,
    server: async () => state.server,
    setting: () => state.setting,
    here: (_serverId, channelId) => state.here === channelId,
    device: () => device,
    holds: () => true,
    toast,
    notify,
    sound,
    open,
    ...overrides,
  });
  return {
    device,
    state,
    toast,
    notify,
    sound,
    open,
    /** Hands one frame over and lets the notice settle. */
    async send(event: SigEvent) {
      handle(event);
      await new Promise((resolve) => setTimeout(resolve, 0));
    },
    advance(ms: number) {
      now += ms;
    },
  };
}

describe("which starts are new", () => {
  it("takes a start event as new, the same Live again as not", () => {
    const starts = createLiveStarts();
    expect(starts.start("srv", "lounge", "ada", 0)).toBe(true);
    expect(starts.start("srv", "lounge", "ada", 20 * MINUTE)).toBe(false);
  });

  it("never takes a Live from a roster for a start, unless its event follows at once", () => {
    const starts = createLiveStarts();
    starts.roster("srv", [{ u: "ada", c: "lounge", s: true }], 0);
    // Opened while Ada was live: a start frame for it later is no news.
    expect(starts.start("srv", "lounge", "ada", LIVE_ROSTER_GRACE_MS)).toBe(
      false,
    );
    // The server can send the roster a moment before the start event.
    starts.roster("srv", [{ u: "bob", c: "lounge", s: true }], MINUTE);
    expect(starts.start("srv", "lounge", "bob", MINUTE + 300)).toBe(true);
  });

  it("does not take a restart within the cooldown after the end for new", () => {
    const starts = createLiveStarts();
    expect(starts.start("srv", "lounge", "ada", 0)).toBe(true);
    // Live for half an hour: the cooldown runs from the end.
    starts.end("srv", "lounge", "ada", 30 * MINUTE);
    expect(starts.start("srv", "lounge", "ada", 39 * MINUTE)).toBe(false);
    starts.end("srv", "lounge", "ada", 39 * MINUTE + 1);
    expect(starts.start("srv", "lounge", "ada", 49 * MINUTE + 1)).toBe(true);
  });

  it("keeps the cooldown when the roster shows the restart before its event", () => {
    const starts = createLiveStarts();
    starts.start("srv", "lounge", "ada", 0);
    starts.end("srv", "lounge", "ada", MINUTE);
    starts.roster("srv", [{ u: "ada", c: "lounge", s: true }], 2 * MINUTE);
    expect(starts.start("srv", "lounge", "ada", 2 * MINUTE + 100)).toBe(false);
  });

  it("starts no cooldown for a Live a roster found gone, but keeps an earlier one", () => {
    const starts = createLiveStarts();
    starts.start("srv", "lounge", "ada", 0);
    // Ended while this device was offline: when is not known.
    starts.roster("srv", [], 3 * 60 * MINUTE);
    expect(starts.start("srv", "lounge", "ada", 3 * 60 * MINUTE + 1)).toBe(
      true,
    );

    starts.end("srv", "lounge", "ada", 4 * 60 * MINUTE);
    starts.roster("srv", [{ u: "ada", c: "lounge", s: true }], 4 * 60 * MINUTE);
    starts.roster("srv", [], 4 * 60 * MINUTE + 1);
    expect(starts.start("srv", "lounge", "ada", 4 * 60 * MINUTE + 2)).toBe(
      false,
    );
  });

  it("counts per server, voice channel and person", () => {
    const starts = createLiveStarts();
    expect(starts.start("srv", "lounge", "ada", 0)).toBe(true);
    expect(starts.start("srv", "bar", "ada", 0)).toBe(true);
    expect(starts.start("srv", "lounge", "bob", 0)).toBe(true);
    expect(starts.start("other", "lounge", "ada", 0)).toBe(true);
  });
});

describe("whether a new Live is announced", () => {
  const hiddenWindow: LiveDevice = {
    toasts: true,
    desktop: true,
    hidden: true,
    focused: false,
    viewingChannelId: undefined,
  };
  const base = {
    start: { serverId: "srv", channelId: "lounge", userId: "ada" },
    me: "me",
    here: false,
    server,
    setting: unset,
    now: 0,
    device: hiddenWindow,
  };

  it("announces someone else's Live where I am not", () => {
    expect(liveNoticeDecision(base)).toEqual({ toast: true, desktop: true });
    expect(
      liveNoticeDecision({ ...base, start: { ...base.start, userId: "me" } }),
    ).toEqual({ toast: false, desktop: false });
    expect(liveNoticeDecision({ ...base, here: true })).toEqual({
      toast: false,
      desktop: false,
    });
  });

  it("follows the server's mute and level: only 'all' announces a Live", () => {
    const decide = (setting: Partial<NotificationSetting>) =>
      liveNoticeDecision({
        ...base,
        now: Date.parse("2026-10-29T12:00:00Z"),
        setting: { ...unset, ...setting },
      });
    expect(decide({ level: "all" }).toast).toBe(true);
    expect(decide({ level: "mentions" }).toast).toBe(false);
    expect(decide({ level: "none" }).toast).toBe(false);
    expect(decide({ muted: true }).toast).toBe(false);
    expect(
      decide({ muted: true, muted_until: "2026-10-29T13:00:00Z" }).toast,
    ).toBe(false);
    // A mute that has run out is over.
    expect(
      decide({ muted: true, muted_until: "2026-10-29T11:00:00Z" }).toast,
    ).toBe(true);
  });

  it("needs a voice channel of the server I may join", () => {
    const none = { toast: false, desktop: false };
    expect(liveNoticeDecision({ ...base, server: undefined })).toEqual(none);
    expect(
      liveNoticeDecision({
        ...base,
        server: { ...server, permissions: ["send_messages"] },
      }),
    ).toEqual(none);
    expect(
      liveNoticeDecision({
        ...base,
        server: { ...server, role: "owner", permissions: [] },
      }).toast,
    ).toBe(true);
    expect(
      liveNoticeDecision({
        ...base,
        start: { ...base.start, channelId: "gone" },
      }),
    ).toEqual(none);
    expect(
      liveNoticeDecision({
        ...base,
        start: { ...base.start, channelId: "chat" },
      }),
    ).toEqual(none);
  });

  it("follows the toast and desktop switches like a message in that channel", () => {
    const device = (patch: Partial<LiveDevice>) =>
      liveNoticeDecision({ ...base, device: { ...base.device, ...patch } });
    expect(device({ toasts: false })).toEqual({ toast: false, desktop: true });
    expect(device({ desktop: false })).toEqual({ toast: true, desktop: false });
    // In front, the toast is enough; the room itself needs neither.
    expect(device({ hidden: false, focused: true })).toEqual({
      toast: true,
      desktop: false,
    });
    expect(
      device({ hidden: false, focused: true, viewingChannelId: "lounge" }),
    ).toEqual({ toast: false, desktop: false });
  });
});

describe("the cue", () => {
  it("sounds with a notification this desktop window showed, never deafened", () => {
    const desk = { mobile: false, deafened: false };
    for (const shown of ["worker", "page", "app"] as const)
      expect(liveSoundWanted(shown, desk)).toBe(true);
    expect(liveSoundWanted("viewed", desk)).toBe(false);
    expect(liveSoundWanted("none", desk)).toBe(false);
    expect(liveSoundWanted("page", { ...desk, deafened: true })).toBe(false);
    expect(liveSoundWanted("worker", { ...desk, mobile: true })).toBe(false);
  });
});

describe("the words", () => {
  it("says who is live where, and only who with content hidden", () => {
    const text = liveText("Ada", "Lounge");
    expect(text).toBe("Ada ist live in Lounge.");
    expect(
      liveNotificationText(false, { author: "Ada", serverName: "Runde", text }),
    ).toEqual({ title: "Ada · Runde", body: "Ada ist live in Lounge." });
    expect(
      liveNotificationText(true, { author: "Ada", serverName: "Runde", text }),
    ).toEqual({ title: "Ada ist live", body: "" });
  });
});

describe("live notices from voice frames", () => {
  it("announces a start event with a toast, a notification and the cue", async () => {
    const h = harness();
    await h.send(start());
    expect(h.toast).toHaveBeenCalledWith({
      channelId: "lounge",
      serverId: "srv",
      dm: false,
      channelLabel: "Runde",
      author: "Ada",
      authorAvatar: "/avatars/ada.png",
      mentioned: false,
      preview: "Ada ist live in Lounge.",
    });
    expect(h.notify).toHaveBeenCalledTimes(1);
    const [notification, onClick] = h.notify.mock.calls[0]!;
    expect(notification).toMatchObject({
      title: "Ada · Runde",
      body: "Ada ist live in Lounge.",
      channelId: "lounge",
      path: "/s/srv/c/lounge",
      user: "me",
    });
    expect(h.sound).toHaveBeenCalledTimes(1);
    onClick();
    expect(h.open).toHaveBeenCalledWith("srv", "lounge");
  });

  it("never announces what a roster or a replay shows", async () => {
    const h = harness();
    await h.send(roster([["ada", "lounge"]]));
    await h.send(start("bob", "bar", { replay: true }));
    h.advance(LIVE_ROSTER_GRACE_MS);
    // A start frame for a Live this device already knew of.
    await h.send(start());
    expect(h.toast).not.toHaveBeenCalled();
    expect(h.notify).not.toHaveBeenCalled();
  });

  it("does not announce my own Live, or one in the channel I sit in", async () => {
    const h = harness();
    await h.send(start("me"));
    h.state.here = "lounge";
    await h.send(start());
    expect(h.toast).not.toHaveBeenCalled();
    // Elsewhere in the server it is news.
    await h.send(start("ada", "bar"));
    expect(h.toast).toHaveBeenCalledTimes(1);
  });

  it("stays quiet for a muted server, and for a Live sitting there unseen", async () => {
    const h = harness();
    h.state.setting = { ...unset, muted: true };
    await h.send(start());
    expect(h.toast).not.toHaveBeenCalled();
    // Unmuted later, the same Live is no news.
    h.state.setting = unset;
    h.advance(MINUTE);
    await h.send(start());
    expect(h.toast).not.toHaveBeenCalled();
  });

  it("announces a reconnect's stop and start once", async () => {
    const h = harness();
    await h.send(start());
    h.advance(MINUTE);
    await h.send(stop());
    h.advance(2_000);
    await h.send(start());
    await h.send({ op: "sig", t: "l", s: "srv", c: "lounge", u: "ada" });
    h.advance(3_000);
    await h.send(start());
    expect(h.toast).toHaveBeenCalledTimes(1);
    expect(h.notify).toHaveBeenCalledTimes(1);
    await h.send(stop());
    h.advance(LIVE_NOTICE_COOLDOWN_MS);
    await h.send(start());
    expect(h.toast).toHaveBeenCalledTimes(2);
  });

  it("plays no cue deafened, without a notification shown, or twice in a burst", async () => {
    const h = harness();
    h.device.deafened = true;
    await h.send(start("ada", "lounge"));
    h.device.deafened = false;
    h.state.shown = "viewed";
    await h.send(start("ada", "bar"));
    expect(h.notify).toHaveBeenCalledTimes(2);
    expect(h.sound).not.toHaveBeenCalled();
    h.state.shown = "page";
    await h.send(start("bob", "lounge"));
    await h.send(start("bob", "bar"));
    expect(h.sound).toHaveBeenCalledTimes(1);
  });

  it("keeps the toast and the notification to their switches", async () => {
    const h = harness();
    h.device.toasts = false;
    await h.send(start("ada", "lounge"));
    expect(h.toast).not.toHaveBeenCalled();
    expect(h.notify).toHaveBeenCalledTimes(1);
    h.device.toasts = true;
    h.device.desktop = false;
    await h.send(start("ada", "bar"));
    expect(h.toast).toHaveBeenCalledTimes(1);
    expect(h.notify).toHaveBeenCalledTimes(1);
    expect(h.sound).toHaveBeenCalledTimes(1);
  });

  it("names only who in a notification that hides content", async () => {
    const h = harness();
    h.device.hideContent = true;
    await h.send(start());
    expect(h.notify.mock.calls[0]?.[0]).toMatchObject({
      title: "Ada ist live",
      body: "",
    });
  });

  it("drops a notice for an account that has gone meanwhile", async () => {
    const h = harness({ holds: () => false });
    await h.send(start());
    expect(h.toast).not.toHaveBeenCalled();
    expect(h.notify).not.toHaveBeenCalled();
  });

  it("gives one start the same notification id in every window, at a cooldown window's edge too", async () => {
    // Both windows hear the start just before a cooldown window ends. The
    // second guesses the server's clock most of a second later, and fetches
    // the server's detail meanwhile.
    let clock = 3 * LIVE_NOTICE_COOLDOWN_MS - 1;
    const first = harness({ now: () => clock, serverNow: () => clock });
    const second = harness({
      now: () => clock,
      serverNow: () => clock + 900,
      server: async () => {
        clock += 1_500;
        return server;
      },
    });
    await first.send(start());
    await second.send(start());
    const id = (h: ReturnType<typeof harness>, call = 0) =>
      h.notify.mock.calls[call]?.[0].messageId;
    expect(id(first)).toBeDefined();
    expect(id(second)).toBe(id(first));
    // A Live after the cooldown is news again, with an id of its own.
    await first.send(stop());
    clock += LIVE_NOTICE_COOLDOWN_MS;
    await first.send(start());
    expect(first.notify).toHaveBeenCalledTimes(2);
    expect(id(first, 1)).not.toBe(id(first));
  });
});
