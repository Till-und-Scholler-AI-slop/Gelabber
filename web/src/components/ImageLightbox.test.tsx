import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { attachmentDownloadUrl, attachmentUrl } from "../messages/api.ts";
import { ImageLightbox } from "./ImageLightbox.tsx";
import { isPlainClick } from "./plainClick.ts";

// Markup only. Esc and the focus on "Schließen" are in
// ImageLightbox.browser.test.tsx; the backdrop click has no test.
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

  it("offers no download for a link preview's picture", () => {
    const preview = renderToStaticMarkup(
      <ImageLightbox
        image={{ id: "preview:x", src: "/api/previews/p/i", filename: "Seite" }}
        onClose={() => {}}
      />,
    );
    expect(preview).toContain('src="/api/previews/p/i"');
    expect(preview).not.toContain("Herunterladen");
  });

  it("shows no stepping for a single picture", () => {
    expect(html).not.toContain("Vorheriges");
    expect(html).not.toContain("Nächstes");
    expect(html).not.toMatch(/\d+ \/ \d+/);
  });
});

// The first render only: stepping itself (← / →, the buttons) is in
// ImageLightbox.browser.test.tsx; a swipe has no test.
describe("image lightbox with a gallery", () => {
  const picture = (id: string, filename: string) => ({
    id,
    src: attachmentUrl(id),
    filename,
    download: attachmentDownloadUrl(id),
  });
  const gallery = [
    picture("a", "erstes.png"),
    picture("b", "zweites.png"),
    picture("c", "drittes.png"),
  ];
  const html = renderToStaticMarkup(
    <ImageLightbox image={gallery[1]!} gallery={gallery} onClose={() => {}} />,
  );

  it("offers previous, next and the count for several pictures", () => {
    expect(html).toMatch(/<button [^>]*aria-label="Vorheriges Bild"/);
    expect(html).toMatch(/<button [^>]*aria-label="Nächstes Bild"/);
    // Read out in the dialog's name already ("Bild 2 von 3").
    expect(html).toMatch(/aria-hidden="true">2 \/ 3</);
  });

  it("opens on the picture asked for, not on the gallery's first", () => {
    expect(html).toContain('aria-label="Bild 2 von 3: zweites.png"');
    expect(html.match(/<img [^>]*>/g)).toHaveLength(1);
    expect(html).toContain('src="/api/attachments/b"');
    expect(html).toContain('alt="zweites.png"');
    expect(html).toContain('href="/api/attachments/b?download=1"');
    expect(html).not.toMatch(/erstes\.png|drittes\.png/);
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
