import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { InstallPanel } from "./InstallApp.tsx";
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

  it("gives iPhone and iPad the Safari steps instead of a button", () => {
    const html = render({ mobile: true, ios: true });
    expect(html).toContain("Öffne Gelabber in Safari.");
    expect(html).toContain("Zum Home-Bildschirm");
    expect(html).not.toContain("<button");
    expect(html).not.toContain("Browsermenü");
  });

  it("reports the states in which nothing can be installed", () => {
    expect(render({ secure: false, mobile: true })).toContain("über HTTPS");
    const installed = render({ installed: true, available: true });
    expect(installed).toContain("bereits als App");
    expect(installed).not.toContain("<button");
    expect(render({ accepted: true, mobile: true })).not.toContain(
      "Browsermenü",
    );
    expect(render({ native: true })).toBe("");
  });
});
