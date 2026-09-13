import { afterEach, describe, expect, it, vi } from "vitest";

import {
  TYPING_BAR_PX,
  isPresenceActivityEvent,
  applyPresence,
  applyPresenceSnap,
  applyTyping,
  formatTyping,
  groupMembers,
  presenceOf,
  pruneTyping,
  resetLiveStores,
  typingUserIds,
  usePresenceStore,
  useTypingStore,
} from "./live.ts";

afterEach(() => {
  resetLiveStores();
});

describe("presence store", () => {
  it("applies updates and snapshots without inventing offline rows", () => {
    applyPresence("s1", "u1", "o");
    applyPresence("s1", "u2", "i");
    expect(presenceOf(usePresenceStore.getState().byServer, "s1", "u1")).toBe(
      "o",
    );
    expect(presenceOf(usePresenceStore.getState().byServer, "s1", "u2")).toBe(
      "i",
    );
    applyPresence("s1", "u1", "x");
    expect(presenceOf(usePresenceStore.getState().byServer, "s1", "u1")).toBe(
      "x",
    );
    applyPresenceSnap("s1", [
      { u: "u2", st: "o" },
      { u: "u3", st: "i" },
    ]);
    expect(usePresenceStore.getState().byServer.s1).toEqual({
      u2: "o",
      u3: "i",
    });
  });

  it("groups members so presence can live beside the chat, not in it", () => {
    const members = [
      { user_id: "b", name: "Bob" },
      { user_id: "a", name: "Ada" },
      { user_id: "c", name: "Cy" },
    ];
    const groups = groupMembers(members, (id) =>
      id === "a" ? "o" : id === "b" ? "i" : "x",
    );
    expect(groups.map((g) => g.group)).toEqual(["o", "i", "x"]);
    expect(groups[0]?.members[0]?.name).toBe("Ada");
    expect(TYPING_BAR_PX).toBe(28);
  });
});

describe("typing store", () => {
  it("starts, stops, and expires by timeout", () => {
    applyTyping("ch", "u1", true, 1_000, 500);
    applyTyping("ch", "u2", true, 1_000, 500);
    expect(typingUserIds(useTypingStore.getState().byChannel, "ch", 1_200)).toEqual(
      ["u1", "u2"],
    );
    applyTyping("ch", "u1", false, 1_200, 500);
    expect(typingUserIds(useTypingStore.getState().byChannel, "ch", 1_200)).toEqual(
      ["u2"],
    );
    pruneTyping(1_600);
    expect(typingUserIds(useTypingStore.getState().byChannel, "ch", 1_600)).toEqual(
      [],
    );
  });

  it("formats German typing copy", () => {
    expect(formatTyping([])).toBe("");
    expect(formatTyping(["Ada"])).toBe("Ada tippt…");
    expect(formatTyping(["Ada", "Bob"])).toBe("Ada und Bob tippen…");
    expect(formatTyping(["Ada", "Bob", "Cy"])).toBe(
      "Ada, Bob und 1 weitere tippen…",
    );
  });

  it("hides the local user from the indicator", () => {
    applyTyping("ch", "me", true, 10, 100);
    applyTyping("ch", "u1", true, 10, 100);
    expect(
      typingUserIds(useTypingStore.getState().byChannel, "ch", 20, "me"),
    ).toEqual(["u1"]);
  });
});

describe("idle activity", () => {
  it("does not treat visibilitychange as activity", () => {
    expect(isPresenceActivityEvent("pointerdown")).toBe(true);
    expect(isPresenceActivityEvent("keydown")).toBe(true);
    expect(isPresenceActivityEvent("mousemove")).toBe(true);
    expect(isPresenceActivityEvent("visibilitychange")).toBe(false);
    expect(isPresenceActivityEvent("focus")).toBe(false);
  });
});

describe("ttl reliability", () => {
  it("does not keep a stale typer after the deadline", () => {
    vi.useFakeTimers();
    applyTyping("ch", "u1", true, Date.now(), 50);
    vi.setSystemTime(Date.now() + 80);
    pruneTyping(Date.now());
    expect(typingUserIds(useTypingStore.getState().byChannel, "ch", Date.now())).toEqual(
      [],
    );
    vi.useRealTimers();
  });
});
