import { describe, expect, it } from "vitest";

import type { DirectMessage } from "../dms/types.ts";
import type { ServerDetail } from "../servers/types.ts";
import {
  mediaContextTopics,
  mergeTopics,
  notificationTopics,
  workspaceTopics,
} from "./useGateway.ts";

const server = {
  id: "srv",
  name: "Home",
  channels: [
    { id: "text-1", kind: "text", name: "general" },
    { id: "voice-1", kind: "voice", name: "Lounge" },
    { id: "text-2", kind: "text", name: "random" },
  ],
} as unknown as ServerDetail;

const dms = [
  { id: "dm-1", peer: { id: "u-b", name: "Bob" } },
  { id: "dm-2", peer: { id: "u-c", name: "Cara" } },
] as unknown as DirectMessage[];

describe("workspace subscribe set", () => {
  it("subscribes a server and its channel", () => {
    expect(workspaceTopics("srv", "ch")).toEqual([
      { s: "srv" },
      { s: "srv", c: "ch" },
    ]);
  });

  it("uses the DM channel id as protocol s", () => {
    expect(workspaceTopics(undefined, "dm-1")).toEqual([
      { s: "dm-1", c: "dm-1" },
    ]);
  });

  it("merges extra channel topics without duplicates", () => {
    expect(
      workspaceTopics("srv", "ch", [
        { s: "srv", c: "ch" },
        { s: "srv", c: "other" },
        { s: "dm-1", c: "dm-1" },
      ]),
    ).toEqual([
      { s: "srv" },
      { s: "srv", c: "ch" },
      { s: "srv", c: "other" },
      { s: "dm-1", c: "dm-1" },
    ]);
  });
});

describe("notification and media topic union", () => {
  it("includes text channels and DMs, not voice channels", () => {
    expect(notificationTopics(server, dms)).toEqual([
      { s: "srv" },
      { s: "srv", c: "text-1" },
      { s: "srv", c: "text-2" },
      { s: "dm-1", c: "dm-1" },
      { s: "dm-2", c: "dm-2" },
    ]);
  });

  it("keeps notification topics when the view has no serverId (settings)", () => {
    const view = workspaceTopics(undefined, undefined);
    const notes = notificationTopics(server, dms);
    const union = mergeTopics(view, notes);
    expect(view).toEqual([]);
    expect(union).toEqual(notes);
    expect(union.some((t) => t.c === "text-2")).toBe(true);
    expect(union.some((t) => t.s === "dm-1")).toBe(true);
  });

  it("unions the active view with notifications without duplicates", () => {
    const view = workspaceTopics("srv", "text-1");
    const notes = notificationTopics(server, dms);
    expect(mergeTopics(view, notes)).toEqual([
      { s: "srv" },
      { s: "srv", c: "text-1" },
      { s: "srv", c: "text-2" },
      { s: "dm-1", c: "dm-1" },
      { s: "dm-2", c: "dm-2" },
    ]);
  });

  it("adds voice and watch context topics", () => {
    expect(
      mediaContextTopics({
        status: "joined",
        serverId: "srv",
        channelId: "voice-1",
        watching: true,
        watchServerId: "srv",
        watchChannelId: "stage",
      }),
    ).toEqual([
      { s: "srv" },
      { s: "srv", c: "voice-1" },
      { s: "srv", c: "stage" },
    ]);
  });

  it("uses the actual watch server independently of the voice server", () => {
    expect(
      mediaContextTopics({
        status: "joined",
        serverId: "voice-server",
        channelId: "voice-room",
        watching: true,
        watchServerId: "watch-server",
        watchChannelId: "stage",
      }),
    ).toEqual([
      { s: "voice-server" },
      { s: "voice-server", c: "voice-room" },
      { s: "watch-server" },
      { s: "watch-server", c: "stage" },
    ]);
  });
});
