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
}));
vi.mock("../voice/session.ts", () => ({
  useVoice: { getState: () => fixture.voice },
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
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain anchor that says where it leads, without the router.
  Link: ({
    to,
    params,
    ...props
  }: {
    to: string;
    params?: Record<string, string>;
  } & Record<string, unknown>) => (
    <a
      data-to={to.replace(
        /\$(\w+)/g,
        (_, name: string) => params?.[name] ?? "",
      )}
      {...props}
    />
  ),
  useNavigate: () => () => Promise.resolve(),
}));

import { useConfirm } from "./confirm.ts";
import { logOut, logoutEndsCall } from "./logout.ts";
import { UserPanel } from "./UserPanel.tsx";
import { ownPresence, panelSubtitle } from "./userPresence.ts";

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
  const lena = {
    id: "u1",
    name: "Lena",
    email: "lena@example.test",
    avatar_url: null,
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
      'data-to="/betrieb/server"',
    );
  });

  it("has no Betrieb for anyone else", () => {
    fixture.session = { user: lena, operator: false };
    expect(menu()).toEqual(["Profil bearbeiten", "Einstellungen", "Abmelden"]);
  });
});
