import { beforeEach, describe, expect, it, vi } from "vitest";

const voice = vi.hoisted(() => ({
  state: {
    status: "idle",
    serverId: null as string | null,
    channelId: null as string | null,
    sourceSubscriptions: {} as Record<string, { s?: boolean }>,
  },
  joinVoice: vi.fn(),
  toggleSourceWatch: vi.fn(),
}));
vi.mock("./session.ts", () => ({
  useVoice: { getState: () => voice.state },
  joinVoice: voice.joinVoice,
  toggleSourceWatch: voice.toggleSourceWatch,
}));

import { currentCall, joinToWatch, watchLabel } from "./watchLive.ts";

const live = {
  serverId: "srv",
  channelId: "sofa",
  channelName: "Sofa",
  people: ["jonas", "mira"],
};

function inCall(channelId: string, watched: string[] = []) {
  voice.state = {
    status: "joined",
    serverId: "srv",
    channelId,
    sourceSubscriptions: Object.fromEntries(
      watched.map((id) => [id, { s: true }]),
    ),
  };
}

describe("joinToWatch", () => {
  beforeEach(() => {
    voice.state = {
      status: "idle",
      serverId: null,
      channelId: null,
      sourceSubscriptions: {},
    };
    voice.joinVoice.mockClear();
    voice.toggleSourceWatch.mockClear();
  });

  it("outside a call joins and watches from the start", () => {
    joinToWatch(live);
    expect(voice.joinVoice).toHaveBeenCalledExactlyOnceWith({
      serverId: "srv",
      channelId: "sofa",
      channelName: "Sofa",
      watch: ["jonas", "mira"],
    });
    expect(voice.toggleSourceWatch).not.toHaveBeenCalled();
    expect(watchLabel(currentCall(voice.state), "srv", "sofa")).toBe(
      "Zuschauen",
    );
  });

  it("in that call watches only the Lives not yet watched", () => {
    inCall("sofa", ["jonas"]);
    joinToWatch(live);
    expect(voice.joinVoice).not.toHaveBeenCalled();
    expect(voice.toggleSourceWatch).toHaveBeenCalledExactlyOnceWith(
      "mira",
      "s",
    );
    expect(watchLabel(currentCall(voice.state), "srv", "sofa")).toBe(
      "Zuschauen",
    );
  });

  it("in another call leaves the switch to the room", () => {
    inCall("kino");
    joinToWatch(live);
    expect(voice.joinVoice).not.toHaveBeenCalled();
    expect(voice.toggleSourceWatch).not.toHaveBeenCalled();
    expect(watchLabel(currentCall(voice.state), "srv", "sofa")).toBe(
      "Zum Kanal",
    );
  });
});
