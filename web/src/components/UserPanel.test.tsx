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
    "aria-label": label,
  }: {
    to: string;
    params?: Record<string, string>;
    children?: ReactNode;
    className?: string;
    title?: string;
    "aria-label"?: string;
  }) => (
    <a
      href={to.replace(/\$(\w+)/g, (_, name: string) => params?.[name] ?? "")}
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
import { UserPanel } from "./UserPanel.tsx";
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

describe("user menu", () => {
  afterEach(() => {
    fixture.session = { user: null, operator: false };
  });

  const menu = () => {
    const html = renderToStaticMarkup(<UserPanel />);
    const nav = html.match(/<nav aria-label="Benutzerkonto">(.*?)<\/nav>/)?.[1];
    return [...(nav ?? "").matchAll(/<(?:a|button)[^>]*>([^<]+)</g)].map(
      ([, label]) => label,
    );
  };
  it("offers Betrieb to an operator, between Einstellungen and Abmelden", () => {
    fixture.session = { user: lena, operator: true };
    expect(menu()).toEqual([
      "Profil bearbeiten",
      "Einstellungen",
      "Betrieb",
      "Abmelden",
    ]);
    expect(renderToStaticMarkup(<UserPanel />)).toContain(
      'href="/betrieb/server"',
    );
  });

  it("has no Betrieb for anyone else", () => {
    fixture.session = { user: lena, operator: false };
    expect(menu()).toEqual(["Profil bearbeiten", "Einstellungen", "Abmelden"]);
  });
});

describe("Feedback geben in the user menu", () => {
  beforeEach(() => {
    fixture.session = { user: lena, operator: true };
  });
  afterEach(() => {
    fixture.session = { user: null, operator: false };
    fixture.feedback = undefined;
  });
  const html = () => renderToStaticMarkup(<UserPanel />);
  const labels = () =>
    [
      ...(
        html().match(/<nav aria-label="Benutzerkonto">(.*?)<\/nav>/)?.[1] ?? ""
      ).matchAll(/<(?:a|button)[^>]*>([^<]+)</g),
    ].map(([, label]) => label);

  it("sits between Einstellungen and Betrieb while feedback is on", () => {
    fixture.feedback = { enabled: true, unread: 0 };
    expect(labels()).toEqual([
      "Profil bearbeiten",
      "Einstellungen",
      "Feedback geben",
      "Betrieb",
      "Abmelden",
    ]);
    expect(html()).toContain(
      '<button type="button" class="user-panel-feedback">Feedback geben</button>',
    );
  });

  it("is missing while feedback is off or its status is not loaded", () => {
    for (const status of [undefined, { enabled: false, unread: 0 }]) {
      fixture.feedback = status;
      expect(labels()).not.toContain("Feedback geben");
      expect(html()).not.toContain("unread-dot");
    }
  });

  it("carries the dot, on the item and on the chevron, while a reply is unread", () => {
    fixture.feedback = { enabled: true, unread: 2 };
    const markup = html();
    expect(markup).toContain(
      'Feedback geben<span class="sr-only">, neue Antwort auf dein Feedback</span><span class="unread-dot" aria-hidden="true"></span></button>',
    );
    expect(markup).toMatch(
      /<summary class="shell-icon-button" aria-label="Benutzermenü, neue Antwort auf dein Feedback"><svg[^]*?<\/svg><span class="unread-dot" aria-hidden="true"><\/span><\/summary>/,
    );
  });

  it("leaves the chevron's name alone without an unread reply", () => {
    fixture.feedback = { enabled: true, unread: 0 };
    expect(html()).toContain(
      '<summary class="shell-icon-button" aria-label="Benutzermenü">',
    );
  });
});

describe("user panel gear", () => {
  beforeEach(() => {
    fixture.session = { user: lena, operator: false };
  });
  afterEach(() => {
    fixture.voice = { status: "idle" };
    fixture.session = { user: null, operator: false };
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
