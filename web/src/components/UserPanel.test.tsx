import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  voice: { status: "idle" },
  leaveVoice: vi.fn(),
  logout: vi.fn(async () => undefined),
}));
vi.mock("../voice/session.ts", () => ({
  useVoice: Object.assign(
    (select: (state: unknown) => unknown) => select(fixture.voice),
    { getState: () => fixture.voice },
  ),
  leaveVoice: fixture.leaveVoice,
}));
vi.mock("../auth/session.ts", () => ({
  logout: fixture.logout,
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "self", name: "Lena", avatar_url: null } }),
  useProfileWrites: (select: (state: unknown) => unknown) =>
    select({ pending: 0 }),
  claimProfileWrite: vi.fn(),
  updateProfile: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  // The link as the page gets it: where it goes and its own attributes.
  Link: ({
    children,
    params,
    className,
    title,
    "aria-label": label,
  }: {
    children: ReactNode;
    params?: { area?: string };
    className?: string;
    title?: string;
    "aria-label"?: string;
  }) => (
    <a
      href={`/settings/${params?.area ?? ""}`}
      className={className}
      aria-label={label}
      title={title}
    >
      {children}
    </a>
  ),
  useNavigate: () => vi.fn(),
}));

import { useConfirm } from "./confirm.ts";
import { logOut, logoutEndsCall } from "./logout.ts";
import { ownPresence, panelSubtitle } from "./userPresence.ts";
import { UserPanel } from "./UserPanel.tsx";

describe("logging out", () => {
  beforeEach(() => {
    fixture.voice = { status: "idle" };
    fixture.leaveVoice.mockReset();
    fixture.logout.mockReset();
  });
  afterEach(() => useConfirm.setState({ pending: null }));

  it("signs out at once when no call would end", async () => {
    await expect(logOut()).resolves.toBe(true);
    expect(useConfirm.getState().pending).toBeNull();
    expect(fixture.logout).toHaveBeenCalledOnce();
  });

  it("asks first during a call, and stays on Abbrechen", async () => {
    fixture.voice = { status: "joined" };
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
    fixture.voice = { status: "joined" };
    const result = logOut();
    useConfirm.getState().answer(useConfirm.getState().pending!.id, true);
    await expect(result).resolves.toBe(true);
    expect(fixture.leaveVoice).toHaveBeenCalledOnce();
    expect(fixture.logout).toHaveBeenCalledOnce();
  });

  it("counts a joined call as a call", () => {
    expect(logoutEndsCall({ status: "joined" })).toBe(true);
    expect(logoutEndsCall({ status: "idle" })).toBe(false);
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

describe("user panel gear", () => {
  afterEach(() => {
    fixture.voice = { status: "idle" };
  });
  /** The panel's controls before the chevron menu: the gear and its kind. */
  const gear = () => {
    const html = renderToStaticMarkup(<UserPanel />);
    return html.slice(0, html.indexOf("<details"));
  };

  it("is the link to the settings outside a call", () => {
    const html = gear();
    expect(html).toMatch(
      /<a href="\/settings\/darstellung" class="shell-icon-button" aria-label="Einstellungen"/,
    );
    expect(html).not.toContain("Anrufeinstellungen");
  });

  it("opens the call settings during a call", () => {
    fixture.voice = { status: "joined" };
    const html = gear();
    expect(html).toMatch(
      /<button type="button" class="shell-icon-button" aria-label="Anrufeinstellungen"[^>]*aria-haspopup="dialog" aria-expanded="false"/,
    );
    expect(html).not.toContain('href="/settings/darstellung"');
    // The app's settings stay in the chevron menu.
    const menu = renderToStaticMarkup(<UserPanel />);
    expect(menu).toMatch(
      /<a href="\/settings\/darstellung">Einstellungen<\/a>/,
    );
  });
});
