import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { attachmentDownloadUrl, attachmentUrl } from "../messages/api.ts";
import { ImageLightbox } from "./ImageLightbox.tsx";
import { isPlainClick } from "./plainClick.ts";

// Markup only: node has no <dialog>, so showModal, Esc (onCancel), the
// backdrop click and the focus on "Schließen" are not exercised here.
describe("image lightbox", () => {
  const html = renderToStaticMarkup(
    <ImageLightbox
      image={{
        id: "b",
        src: attachmentUrl("b"),
        filename: "zweites.png",
        download: attachmentDownloadUrl("b"),
      }}
      onClose={() => {}}
    />,
  );

  it("shows the picture from its display link, named", () => {
    expect(html).toContain('src="/api/attachments/b"');
    expect(html).toContain('alt="zweites.png"');
    expect(html).toContain('aria-label="zweites.png"');
    expect(html).toContain('aria-label="Schließen"');
  });

  it("saves through the download variant, not the display link", () => {
    expect(html).toContain('href="/api/attachments/b?download=1"');
    expect(html).toContain('download="zweites.png"');
  });

  it("has no stepping: a message carries one attachment", () => {
    expect(html).not.toContain("Vorheriges");
    expect(html).not.toContain("Nächstes");
    expect(html).not.toContain("lr-lightbox-count");
  });
});

describe("plain click", () => {
  const click = {
    button: 0,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
  };

  it("takes a plain primary click", () => {
    expect(isPlainClick(click)).toBe(true);
  });

  it("leaves new tab, window, save and middle click to the browser", () => {
    expect(isPlainClick({ ...click, ctrlKey: true })).toBe(false);
    expect(isPlainClick({ ...click, metaKey: true })).toBe(false);
    expect(isPlainClick({ ...click, shiftKey: true })).toBe(false);
    expect(isPlainClick({ ...click, altKey: true })).toBe(false);
    expect(isPlainClick({ ...click, button: 1 })).toBe(false);
  });
});
