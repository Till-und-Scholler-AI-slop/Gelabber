import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { InstallDisclosurePanel, InstallPanel } from "./InstallApp.tsx";
import { useInstallation, type InstallState } from "./install.ts";

function render(state: Partial<InstallState>) {
  return renderToStaticMarkup(
    <InstallPanel
      state={{ ...useInstallation.getInitialState(), secure: true, ...state }}
    />,
  );
}

describe("install panel", () => {
  it("offers the browser prompt and only then calls the menu an alternative", () => {
    const offered = render({ mobile: true, available: true });
    expect(offered).toContain(">App installieren</button>");
    expect(offered).toContain("Du kannst auch im Browsermenü");
    const waiting = render({ mobile: true });
    expect(waiting).not.toContain("<button");
    expect(waiting).toContain("Wähle im Browsermenü");
    expect(waiting).not.toContain("Du kannst auch");
  });

  it("keeps phone wording away from desktop browsers", () => {
    const desktop = render({});
    expect(desktop).toContain("In einem eigenen Fenster öffnen.");
    expect(desktop).toContain("Adressleiste");
    expect(desktop).not.toContain("Startbildschirm");
    expect(desktop).not.toContain("Safari");
    expect(render({ available: true })).not.toContain("Adressleiste");
  });

  it("names where each desktop browser installs, or that it cannot", () => {
    expect(render({ browser: "chromium" })).toContain(
      "Klicke rechts in der Adressleiste auf das Symbol zum Installieren.",
    );
    expect(render({ browser: "safari" })).toContain(
      "„Ablage“ › „Zum Dock hinzufügen“",
    );
    const firefox = render({ browser: "firefox", desktop: "windows" });
    expect(firefox).toContain("Firefox kann keine Web-Apps installieren.");
    expect(firefox).toContain("Desktop-App für Windows");
    expect(firefox).not.toContain("Adressleiste");
    // macOS has no desktop app to point to.
    const mac = render({ browser: "firefox" });
    expect(mac).toContain("in Chrome oder Edge");
    expect(mac).not.toContain("Desktop-App");
    expect(render({ browser: "other" })).toContain(
      "in der Adressleiste oder im Browsermenü",
    );
  });

  it("gives iPhone and iPad the Safari steps instead of a button", () => {
    const html = render({ mobile: true, ios: true });
    expect(html).toContain("Öffne Gelabber in Safari.");
    expect(html).toContain("Zum Home-Bildschirm");
    expect(html).not.toContain("<button");
    expect(html).not.toContain("Browsermenü");
  });

  it("reports the states in which nothing can be installed", () => {
    const insecure = render({ secure: false, browser: "chromium" });
    expect(insecure).toContain(
      "Zum Installieren braucht es HTTPS. Bitte die Person, die diese Instanz betreibt, das einzurichten.",
    );
    expect(insecure).not.toContain("Adressleiste");
    const installed = render({ installed: true, available: true });
    expect(installed).toContain("bereits als App");
    expect(installed).not.toContain("<button");
    expect(render({ accepted: true, mobile: true })).not.toContain(
      "Browsermenü",
    );
  });

  it("folds into one line on the sign-in page", () => {
    const state = {
      ...useInstallation.getInitialState(),
      secure: true,
      mobile: true,
    };
    const html = renderToStaticMarkup(<InstallDisclosurePanel state={state} />);
    expect(html).toContain(
      '<details class="pwa-install-disclosure"><summary>Gelabber als App installieren<svg',
    );
    expect(html).toContain('<section class="pwa-install"');
    expect(html).not.toContain("<details open");
  });
});
