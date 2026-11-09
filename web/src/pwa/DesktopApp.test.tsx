import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { DesktopAppPanel } from "./DesktopApp.tsx";
import {
  ARCH_INSTALL_COMMAND,
  LINUX_TARBALL_URL,
  WINDOWS_SETUP_URL,
} from "./desktopApp.ts";

const SERVER = "https://gelabber.example";

function render(platform: "windows" | "linux") {
  return renderToStaticMarkup(
    <DesktopAppPanel platform={platform} server={SERVER} />,
  );
}

/** The part before the other system's disclosure. */
function own(html: string) {
  return html.split("<details", 1)[0]!;
}

describe("desktop app panel", () => {
  it("puts the Windows installer first on Windows, Linux behind one line", () => {
    const html = render("windows");
    expect(own(html)).toContain("Desktop-App für Windows");
    expect(own(html)).toContain(`href="${WINDOWS_SETUP_URL}"`);
    expect(own(html)).toContain("Trotzdem ausführen");
    expect(own(html)).not.toContain("pacman");
    expect(html).toContain("<summary>Für Linux");
    expect(html).toContain(ARCH_INSTALL_COMMAND);
  });

  it("puts the Arch command and the tarball first on Linux", () => {
    const html = render("linux");
    expect(own(html)).toContain("Desktop-App für Linux");
    expect(own(html)).toContain(`<code>${ARCH_INSTALL_COMMAND}</code>`);
    expect(own(html)).toContain(">Kopieren<");
    expect(own(html)).toContain("sudo pacman -Syu");
    expect(own(html)).toContain(`href="${LINUX_TARBALL_URL}"`);
    expect(own(html)).not.toContain(WINDOWS_SETUP_URL);
    expect(html).toContain("<summary>Für Windows");
    expect(html).toContain(`href="${WINDOWS_SETUP_URL}"`);
  });

  it("names this server for the app's first start", () => {
    for (const platform of ["windows", "linux"] as const) {
      expect(own(render(platform))).toContain(`<code>${SERVER}</code>`);
    }
  });
});
