import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  voice: { status: "idle", watching: false },
  leaveVoice: vi.fn(),
  stopWatching: vi.fn(),
  logout: vi.fn(async () => undefined),
}));
vi.mock("../voice/session.ts", () => ({
  useVoice: { getState: () => fixture.voice },
  leaveVoice: fixture.leaveVoice,
  stopWatching: fixture.stopWatching,
}));
vi.mock("../auth/session.ts", () => ({ logout: fixture.logout }));

import { useConfirm } from "./confirm.ts";
import { logOut, logoutEndsCall } from "./logout.ts";
import { ownPresence, panelSubtitle } from "./userPresence.ts";

describe("logging out", () => {
  beforeEach(() => {
    fixture.voice = { status: "idle", watching: false };
    fixture.leaveVoice.mockReset();
    fixture.stopWatching.mockReset();
    fixture.logout.mockReset();
  });
  afterEach(() => useConfirm.setState({ pending: null }));

  it("signs out at once when no call would end", async () => {
    await expect(logOut()).resolves.toBe(true);
    expect(useConfirm.getState().pending).toBeNull();
    expect(fixture.logout).toHaveBeenCalledOnce();
  });

  it("asks first during a call, and stays on Abbrechen", async () => {
    fixture.voice = { status: "joined", watching: false };
    const result = logOut();
    const pending = useConfirm.getState().pending;
    expect(pending?.title).toBe("Abmelden und Anruf beenden?");
    expect(pending?.confirmLabel).toBe("Abmelden");
    useConfirm.getState().answer(pending!.id, false);
    await expect(result).resolves.toBe(false);
    expect(fixture.leaveVoice).not.toHaveBeenCalled();
    expect(fixture.logout).not.toHaveBeenCalled();
  });

  it("leaves the call and signs out once confirmed", async () => {
    fixture.voice = { status: "idle", watching: true };
    const result = logOut();
    useConfirm.getState().answer(useConfirm.getState().pending!.id, true);
    await expect(result).resolves.toBe(true);
    expect(fixture.leaveVoice).toHaveBeenCalledOnce();
    expect(fixture.stopWatching).toHaveBeenCalledOnce();
    expect(fixture.logout).toHaveBeenCalledOnce();
  });

  it("counts a joined call and a watched stream as a call", () => {
    expect(logoutEndsCall({ status: "joined", watching: false })).toBe(true);
    expect(logoutEndsCall({ status: "idle", watching: true })).toBe(true);
    expect(logoutEndsCall({ status: "idle", watching: false })).toBe(false);
  });
});

describe("user panel subtitle", () => {
  it("always says the presence, never 'Dein Profil'", () => {
    expect(panelSubtitle("o", null)).toBe("Online");
    expect(panelSubtitle("i", "")).toBe("Abwesend");
    expect(panelSubtitle("o", "Zocke gerade")).toBe("Online · Zocke gerade");
  });

  it("takes the presence of the server in view, else of any server", () => {
    const byServer = { a: { me: "i" as const }, b: { me: "o" as const } };
    expect(ownPresence(byServer, "me", "b")).toBe("o");
    expect(ownPresence(byServer, "me", undefined)).toBe("i");
    expect(ownPresence({}, "me", undefined)).toBe("o");
  });
});
