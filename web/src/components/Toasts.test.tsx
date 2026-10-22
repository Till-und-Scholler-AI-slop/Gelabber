import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import { ToastRegions, Toasts } from "./Toasts.tsx";
import { clearToasts, notify, useToasts } from "./toasts.ts";

describe("toast markup", () => {
  afterEach(clearToasts);

  it("keeps both live regions in the page with no toast", () => {
    const html = renderToStaticMarkup(<Toasts />);
    expect(html).toContain('role="status"');
    expect(html).toContain('role="alert"');
  });

  it("puts notices in the polite region and errors in the alert", () => {
    notify("Einladungslink kopiert.");
    useToasts.getState().push("error", "Speichern fehlgeschlagen.");
    const html = renderToStaticMarkup(
      <ToastRegions toasts={useToasts.getState().toasts} />,
    );
    const status = html.indexOf('role="status"');
    const alert = html.indexOf('role="alert"');
    expect(html.indexOf("Einladungslink kopiert.")).toBeGreaterThan(status);
    expect(html.indexOf("Einladungslink kopiert.")).toBeLessThan(alert);
    expect(html.indexOf("Speichern fehlgeschlagen.")).toBeGreaterThan(alert);
  });

  it("is a card with a separate close button and an optional action", () => {
    notify("Link widerrufen", {
      action: { label: "Rückgängig", onClick: () => {} },
    });
    const html = renderToStaticMarkup(
      <ToastRegions toasts={useToasts.getState().toasts} />,
    );
    expect(html).toContain('<p class="ui-toast__message">Link widerrufen</p>');
    expect(html).toMatch(/<button[^>]*>Rückgängig<\/button>/);
    expect(html).toContain('aria-label="Schließen"');
  });
});
