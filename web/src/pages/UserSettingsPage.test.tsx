import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  params: { area: "darstellung" },
  feedback: undefined as { enabled: boolean; unread: number } | undefined,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#">{children}</a>,
  Navigate: () => null,
  useCanGoBack: () => false,
  useLocation: () => undefined,
  useNavigate: () => vi.fn(),
  useParams: () => fixture.params,
  useRouter: () => ({}),
}));
// The areas' own content has its own tests; here only the frame counts.
vi.mock("../theme/ThemesPanel.tsx", () => ({ ThemesPanel: () => null }));
vi.mock("../voice/VoiceSettings.tsx", () => ({
  MediaSettingsForm: () => null,
}));
vi.mock("./ProfilePage.tsx", () => ({
  ProfileArea: () => null,
  ProfileCardArea: () => null,
}));
vi.mock("./SecurityArea.tsx", () => ({ SecurityArea: () => null }));
vi.mock("../components/UserPanel.tsx", () => ({ UserPanel: () => null }));
vi.mock("../feedback/FeedbackArea.tsx", () => ({ FeedbackArea: () => null }));
vi.mock("../feedback/queries.ts", () => ({
  useFeedbackStatus: () => ({ data: fixture.feedback }),
}));
vi.mock("../messages/notificationPrivacy.ts", () => ({
  useNotificationPrivacy: () => ({ data: { hide_content: false } }),
  useChangeNotificationPrivacy: () => vi.fn(),
}));

import { setLocale } from "../i18n/locale.ts";
import { SettingsAreaPart } from "./settingsAreaPart.ts";
import { SettingsSidebar, UserSettingsPage } from "./UserSettingsPage.tsx";

const page = () => renderToStaticMarkup(<UserSettingsPage />);

describe("user settings page", () => {
  // The area forms are a chunk of their own; loaded, they render at once.
  beforeAll(() => SettingsAreaPart.preload());
  beforeEach(() => {
    fixture.params = { area: "darstellung" };
    fixture.feedback = undefined;
  });

  it("titles the area with a heading that takes focus", () => {
    // The page moves focus here whenever the area changes.
    expect(page()).toContain(
      '<h2 id="settings-area-heading" tabindex="-1">Darstellung</h2>',
    );
    fixture.params = { area: "audio" };
    expect(page()).toContain('tabindex="-1">Audio</h2>');
  });

  it("tags where settings are kept instead of one footnote", () => {
    const appearance = page();
    expect(appearance).toContain("Für dein Konto");
    expect(appearance).toContain("Nur dieses Gerät");
    expect(appearance).not.toContain("automatisch in diesem Browser");

    fixture.params = { area: "audio" };
    expect(page()).toContain("Nur dieses Gerät");
    fixture.params = { area: "themes" };
    expect(page()).toContain("Für dein Konto");
  });

  it("puts message notifications next to the account's lock-screen switch", () => {
    fixture.params = { area: "benachrichtigungen" };
    const html = page();
    expect(html).toContain('id="message-toasts"');
    expect(html).toContain("Inhalt in Benachrichtigungen verbergen");
    expect(html.indexOf("Nur dieses Gerät")).toBeLessThan(
      html.indexOf("Für dein Konto"),
    );
  });

  it("offers the text size, compact first", () => {
    const html = page();
    expect(html).toContain('aria-label="Textgröße"');
    expect(html).toMatch(/aria-checked="true"[^>]*>Kompakt</);
  });

  it("speaks English when the interface does", () => {
    setLocale("en", { persist: false });
    try {
      const html = page();
      expect(html).toContain('tabindex="-1">Appearance</h2>');
      expect(html).toContain(
        "<p>Language, colour scheme, text size and motion.</p>",
      );
      expect(html).toContain("For your account");
      expect(html).toContain("This device only");
      expect(html).toContain("Colour scheme");
      const sidebar = renderToStaticMarkup(<SettingsSidebar />);
      expect(sidebar).toContain(">Settings<");
      expect(sidebar.indexOf(">Account<")).toBeLessThan(
        sidebar.indexOf(">Profile card<"),
      );
      expect(sidebar).toContain(">Notifications<");
    } finally {
      setLocale("de", { persist: false });
    }
  });

  it("groups the sidebar into Konto and App", () => {
    const html = renderToStaticMarkup(<SettingsSidebar />);
    expect(html.indexOf(">Konto<")).toBeLessThan(html.indexOf(">Profil<"));
    expect(html.indexOf(">Sicherheit<")).toBeLessThan(html.indexOf(">App<"));
    expect(html.indexOf(">App<")).toBeLessThan(html.indexOf(">Darstellung<"));
  });

  it("lists Feedback under Konto only while the instance takes it", () => {
    const sidebar = () => renderToStaticMarkup(<SettingsSidebar />);
    expect(sidebar()).not.toContain(">Feedback<");
    fixture.feedback = { enabled: false, unread: 0 };
    expect(sidebar()).not.toContain(">Feedback<");

    fixture.feedback = { enabled: true, unread: 0 };
    const html = sidebar();
    expect(html.indexOf(">Sicherheit<")).toBeLessThan(
      html.indexOf(">Feedback<"),
    );
    expect(html.indexOf(">Feedback<")).toBeLessThan(html.indexOf(">App<"));
    expect(html).not.toContain("unread-dot");
  });

  it("puts a dot on the Feedback row while a reply is unread", () => {
    fixture.feedback = { enabled: true, unread: 2 };
    const html = renderToStaticMarkup(<SettingsSidebar />);
    expect(html).toMatch(
      /<span>Feedback<\/span><span class="sr-only">, neue Antwort<\/span><span class="unread-dot" aria-hidden="true"><\/span>/,
    );
    expect(html.match(/unread-dot/g)).toHaveLength(1);
  });

  it("titles the Feedback area like every other", () => {
    fixture.feedback = { enabled: true, unread: 0 };
    fixture.params = { area: "feedback" };
    const html = page();
    expect(html).toContain('tabindex="-1">Feedback</h2>');
    expect(html).toContain(
      "<p>Was du geschickt hast und die Antworten darauf.</p>",
    );
    expect(html).toContain("Für dein Konto");
  });
});
