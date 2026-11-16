import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  voice: { status: "idle" },
  leaveVoice: vi.fn(),
  logout: vi.fn(async () => undefined),
  session: {
    user: null as Record<string, unknown> | null,
    operator: false,
  },
  feedback: undefined as { enabled: boolean; unread: number } | undefined,
}));
vi.mock("../voice/session.ts", () => ({
  useVoice: Object.assign(
    (select: (state: unknown) => unknown) => select(fixture.voice),
    { getState: () => fixture.voice },
  ),
  leaveVoice: fixture.leaveVoice,
}));
// A server render reads a store's initial state: the session comes from here.
vi.mock("../auth/session.ts", () => ({
  logout: fixture.logout,
  useSession: (select: (state: typeof fixture.session) => unknown) =>
    select(fixture.session),
  useProfileWrites: (select: (state: { pending: number }) => unknown) =>
    select({ pending: 0 }),
  claimProfileWrite: () => null,
  updateProfile: vi.fn(),
}));
vi.mock("../feedback/queries.ts", () => ({
  useFeedbackStatus: () => ({ data: fixture.feedback }),
}));
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain anchor that says where it leads, without the router: where it
  // goes and its own attributes.
  Link: ({
    to,
    params,
    children,
    className,
    title,
    role,
    "aria-label": label,
  }: {
    to: string;
    params?: Record<string, string>;
    children?: ReactNode;
    className?: string;
    title?: string;
    role?: string;
    "aria-label"?: string;
  }) => (
    <a
      href={to.replace(/\$(\w+)/g, (_, name: string) => params?.[name] ?? "")}
      role={role}
      className={className}
      aria-label={label}
      title={title}
    >
      {children}
    </a>
  ),
  useNavigate: () => () => Promise.resolve(),
}));

import { useConfirm } from "./confirm.ts";
import { logOut, logoutEndsCall } from "./logout.ts";
import { accountStep } from "./accountKeys.ts";
import { AccountMenu, UserPanel } from "./UserPanel.tsx";
import { ownPresence, panelSubtitle } from "./userPresence.ts";

const lena = {
  id: "u1",
  name: "Lena",
  email: "lena@example.test",
  avatar_url: null,
};

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

/** The account menu's entries (the popover's lower part), as rendered. */
const menuMarkup = (feedback = false, unread = false) =>
  renderToStaticMarkup(
    <AccountMenu
      feedback={feedback}
      unread={unread}
      onDone={() => undefined}
      onFeedback={() => undefined}
    />,
  );
const menuLabels = (markup: string) =>
  [...markup.matchAll(/<(?:a|button)[^>]*role="menuitem"[^>]*>([^<]+)</g)].map(
    ([, label]) => label,
  );

describe("account menu", () => {
  afterEach(() => {
    fixture.session = { user: null, operator: false };
  });

  it("is a menu of the account's entries, without Einstellungen", () => {
    fixture.session = { user: lena, operator: false };
    const markup = menuMarkup();
    expect(markup).toMatch(/^<div role="menu" aria-label="Benutzermenü"/);
    expect(menuLabels(markup)).toEqual(["Profil bearbeiten", "Abmelden"]);
    // A line under the status card, and one before signing out.
    expect(markup.match(/<hr class="shell-menu-separator"\/>/g)).toHaveLength(
      2,
    );
    expect(markup).toContain('href="/settings/profil"');
  });

  it("offers Betrieb to an operator, before Abmelden", () => {
    fixture.session = { user: lena, operator: true };
    const markup = menuMarkup();
    expect(menuLabels(markup)).toEqual([
      "Profil bearbeiten",
      "Betrieb",
      "Abmelden",
    ]);
    expect(markup).toContain('href="/betrieb/server"');
  });

  it("puts Feedback geben after the profile while feedback is on", () => {
    fixture.session = { user: lena, operator: true };
    expect(menuLabels(menuMarkup(true))).toEqual([
      "Profil bearbeiten",
      "Feedback geben",
      "Betrieb",
      "Abmelden",
    ]);
    expect(menuMarkup(true)).not.toContain("unread-dot");
    expect(menuMarkup(true, true)).toContain(
      'Feedback geben<span class="sr-only">, neue Antwort auf dein Feedback</span><span class="unread-dot" aria-hidden="true"></span></button>',
    );
  });

  it("starts with the presence modes, the current one checked (#590)", () => {
    fixture.session = { user: lena, operator: false };
    // A server render reads the store's first state: automatic.
    const markup = menuMarkup();
    const radios = [
      ...markup.matchAll(
        /role="menuitemradio" aria-checked="(true|false)"[^>]*data-presence-mode="(\w+)"/g,
      ),
    ].map(([, checked, mode]) => [mode, checked]);
    expect(radios).toEqual([
      ["auto", "true"],
      ["idle", "false"],
      ["dnd", "false"],
      ["invisible", "false"],
    ]);
    // Away and the others ask for how long first; automatic does not.
    expect(markup.match(/aria-expanded="false"/g)).toHaveLength(3);
    expect(markup).toContain('<span class="presence-mode__dot gel-presence-d"');
    // The modes come before the account's entries.
    expect(markup.indexOf("Nicht stören")).toBeLessThan(
      markup.indexOf("Profil bearbeiten"),
    );
  });

  it("moves with the arrows, round the ends, and up into the status field", () => {
    expect(accountStep("ArrowDown", 0, 3)).toBe(1);
    expect(accountStep("ArrowDown", 2, 3)).toBe(0);
    expect(accountStep("ArrowUp", 2, 3)).toBe(1);
    // -1: the status field above the menu.
    expect(accountStep("ArrowUp", 0, 3)).toBe(-1);
    expect(accountStep("Home", 2, 3)).toBe(0);
    expect(accountStep("End", 0, 3)).toBe(2);
    expect(accountStep("Enter", 0, 3)).toBeNull();
    expect(accountStep("ArrowLeft", 0, 3)).toBeNull();
  });
});

describe("user panel", () => {
  beforeEach(() => {
    fixture.session = { user: lena, operator: false };
  });
  afterEach(() => {
    fixture.voice = { status: "idle" };
    fixture.session = { user: null, operator: false };
    fixture.feedback = undefined;
  });
  const html = () => renderToStaticMarkup(<UserPanel />);

  it("has the avatar's popover and the gear, and no chevron", () => {
    const markup = html();
    expect(markup).not.toContain("<details");
    expect(markup).not.toContain("<summary");
    expect(markup).toMatch(
      /<button type="button" class="user-panel-profile" aria-haspopup="dialog" aria-expanded="false">/,
    );
  });

  it("opens Settings from the gear, in a call too", () => {
    for (const status of ["idle", "joined"]) {
      fixture.voice = { status };
      const markup = html();
      expect(markup).toMatch(
        /<a href="\/settings\/darstellung" class="shell-icon-button" aria-label="Einstellungen"/,
      );
      expect(markup).not.toContain("Anrufeinstellungen");
    }
  });

  it("carries an unread reply's dot on the avatar, and says so", () => {
    fixture.feedback = { enabled: true, unread: 2 };
    const markup = html();
    expect(markup).toMatch(
      /<span class="user-panel-avatar" aria-hidden="true">[^]*?<span class="unread-dot" aria-hidden="true"><\/span><\/span>/,
    );
    // Inside the avatar's button, so its name ("Lena Online, neue Antwort
    // …") ends with it; the avatar's presence label stays out of the name.
    const button = markup.slice(0, markup.indexOf("</button>"));
    expect(button).toContain(
      '<span class="sr-only">, neue Antwort auf dein Feedback</span>',
    );
  });

  it("has no dot while every reply is read or feedback is off", () => {
    for (const status of [
      undefined,
      { enabled: false, unread: 3 },
      { enabled: true, unread: 0 },
    ]) {
      fixture.feedback = status;
      expect(html()).not.toContain("unread-dot");
      expect(html()).not.toContain("neue Antwort");
    }
  });
});
