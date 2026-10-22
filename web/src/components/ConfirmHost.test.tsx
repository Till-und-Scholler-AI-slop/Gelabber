import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ConfirmDialog } from "./ConfirmHost.tsx";

// Markup only: node has no <dialog>, so showModal, Escape (onCancel), the
// backdrop click and the focus return come from the shared `Modal` and the
// browser. The answers themselves are covered in confirm.test.ts.
function render(request: {
  title: string;
  message?: string;
  confirmLabel: string;
  tone?: "danger" | "neutral";
}) {
  return renderToStaticMarkup(
    <ConfirmDialog pending={{ ...request, id: 1 }} answer={() => {}} />,
  );
}

function button(html: string, label: string) {
  const match = [...html.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].find(
    ([, text]) => text === label,
  );
  expect(match, `button ${label}`).toBeDefined();
  return match![0];
}

describe("confirm dialog markup", () => {
  it("is an alertdialog described by its message", () => {
    const html = render({
      title: "Server löschen?",
      message: "Es gibt kein Zurück.",
      confirmLabel: "Server löschen",
    });
    expect(html).toContain('role="alertdialog"');
    const describedBy = /aria-describedby="([^"]+)"/.exec(html)?.[1];
    expect(describedBy).toBeTruthy();
    expect(html).toContain(`<p id="${describedBy}"`);
    expect(html).toContain("Es gibt kein Zurück.");
  });

  it("has no description reference without a message", () => {
    const html = render({ title: "Verlassen?", confirmLabel: "Verlassen" });
    expect(html).toContain('role="alertdialog"');
    expect(html).not.toContain("aria-describedby");
  });

  it("opens a destructive question on Abbrechen, not on the action", () => {
    const html = render({
      title: "Nachricht löschen?",
      confirmLabel: "Löschen",
    });
    expect(button(html, "Abbrechen")).toContain("autofocus");
    expect(button(html, "Löschen")).not.toContain("autofocus");
    expect(button(html, "Löschen")).toContain('type="submit"');
    expect(button(html, "Löschen")).toContain("ui-button--danger");
  });

  it("opens a neutral question on its confirm button", () => {
    const html = render({
      title: "Wieder zulassen?",
      confirmLabel: "Entsperren",
      tone: "neutral",
    });
    expect(button(html, "Entsperren")).toContain("autofocus");
    expect(button(html, "Entsperren")).toContain("ui-button--primary");
    expect(button(html, "Abbrechen")).not.toContain("autofocus");
  });
});
