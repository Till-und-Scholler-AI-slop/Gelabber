import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { RouteErrorView } from "./RouteErrorPage.tsx";

describe("route error page", () => {
  it("says in its own sentence that reloading ends the call", () => {
    const html = renderToStaticMarkup(<RouteErrorView inCall />);
    expect(html).toContain("Dein Anruf läuft weiter. Neu laden beendet ihn.");
  });
});
