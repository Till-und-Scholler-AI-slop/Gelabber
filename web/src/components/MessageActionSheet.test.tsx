import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MessageActionItems } from "./MessageActionSheet.tsx";

const noop = () => {};
const labels = (html: string) =>
  [...html.matchAll(/role="menuitem"[^>]*>(?:<svg.*?<\/svg>)?([^<]+)</g)].map(
    (match) => match[1],
  );

describe("message action sheet", () => {
  it("offers quick reactions, then reply, edit, copy and delete last", () => {
    const html = renderToStaticMarkup(
      <MessageActionItems
        actions={{
          react: noop,
          moreReactions: noop,
          reply: noop,
          edit: noop,
          copy: noop,
          remove: noop,
        }}
        onDone={noop}
      />,
    );
    for (const emoji of ["👍", "❤️", "😂", "😮", "😢", "🎉"])
      expect(html).toContain(`aria-label="Mit ${emoji} reagieren"`);
    expect(html).toContain('aria-label="Andere Reaktion auswählen"');
    expect(labels(html)).toEqual([
      "Antworten",
      "Bearbeiten",
      "Text kopieren",
      "Löschen",
    ]);
    expect(html).toContain('title="Löschen (Umschalt: ohne Nachfrage)"');
  });

  it("leaves out what the viewer may not do", () => {
    const html = renderToStaticMarkup(
      <MessageActionItems actions={{ copy: noop }} onDone={noop} />,
    );
    expect(labels(html)).toEqual(["Text kopieren"]);
    expect(html).not.toContain("reagieren");
    expect(html).not.toContain("<hr");
  });

  it("marks a reaction already given as one to take back", () => {
    const html = renderToStaticMarkup(
      <MessageActionItems
        actions={{ react: noop }}
        mine={["👍"]}
        onDone={noop}
      />,
    );
    expect(html).toContain('aria-label="Reaktion 👍 entfernen"');
    expect(html).toContain('aria-label="Mit ❤️ reagieren"');
  });
});
