import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ImageLightbox } from "./ImageLightbox.tsx";

const images = [
  { id: "a", src: "/a.png", filename: "erstes.png" },
  { id: "b", src: "/b.png", filename: "zweites.png" },
  { id: "c", src: "/c.png", filename: "drittes.png" },
];
const render = (list: typeof images, index: number) =>
  renderToStaticMarkup(
    <ImageLightbox
      images={list}
      index={index}
      downloadUrl={(id) => `/api/attachments/${id}?download=1`}
      onIndex={() => {}}
      onClose={() => {}}
    />,
  );

describe("image lightbox", () => {
  it("shows the chosen image with its name, download and position", () => {
    const html = render(images, 1);
    expect(html).toContain('src="/b.png"');
    expect(html).toContain('aria-label="zweites.png"');
    expect(html).toContain('href="/api/attachments/b?download=1"');
    expect(html).toContain('download="zweites.png"');
    expect(html).toContain("2 / 3");
    expect(html).toContain("Schließen");
  });

  it("has no stepping for a single image and renders nothing out of range", () => {
    const single = render(images.slice(0, 1), 0);
    expect(single).not.toContain("1 / 1");
    expect(single).not.toContain("Vorheriges");
    expect(render(images, 5)).toBe("");
  });
});
