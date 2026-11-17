import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";

const actions = vi.hoisted(() => ({ voice: false }));
vi.mock("./notificationActions.ts", async (original) => ({
  ...(await original<typeof import("./notificationActions.ts")>()),
  useNotificationActions: () => ({
    own: { level: null, muted: false, muted_until: null },
    muted: false,
    unread: false,
    serverMuted: false,
    inherited: "all",
    isLevel: (level: string | null) => level === "all",
    voice: actions.voice,
    setVoice: vi.fn(),
    mute: vi.fn(),
    unmute: vi.fn(),
    setLevel: vi.fn(),
    markRead: vi.fn(),
    openSettings: vi.fn(),
  }),
}));

import { NotificationSettingsForm } from "./NotificationSettingsDialog.tsx";
import type { NotificationTarget } from "./notificationTarget.ts";

function render(target: NotificationTarget) {
  return renderToStaticMarkup(
    <NotificationSettingsForm target={target} onDone={() => {}} />,
  );
}

afterEach(() => {
  actions.voice = false;
  setLocale("de", { persist: false });
});

describe("the voice push in the notification dialog (#616)", () => {
  const server: NotificationTarget = {
    scope: "server",
    id: "s1",
    name: "Wohnzimmer",
  };

  it("is a switch of the server, off unless asked for", () => {
    const html = render(server);
    expect(html).toContain("Sprachkanäle");
    expect(html).toContain("Push, wenn ein Anruf beginnt");
    expect(html).toMatch(/<input[^>]*role="switch"/);
    expect(html).not.toMatch(/<input[^>]*role="switch"[^>]*checked=""/);

    actions.voice = true;
    expect(render(server)).toMatch(/<input[^>]*role="switch"[^>]*checked=""/);
  });

  it("is not offered for a channel or a direct message", () => {
    for (const target of [
      { scope: "channel", id: "c1", name: "#allgemein", serverId: "s1" },
      { scope: "channel", id: "d1", name: "Jonas", dm: true },
    ] satisfies NotificationTarget[]) {
      const html = render(target);
      expect(html).toContain("Stummschalten");
      expect(html).not.toContain('role="switch"');
    }
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    const html = render(server);
    expect(html).toContain("Voice channels");
    expect(html).toContain("Push when a call begins");
  });
});
