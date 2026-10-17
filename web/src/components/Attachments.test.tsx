import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { Attachment } from "../messages/types.ts";
import { AttachmentList } from "./Attachments.tsx";

const file = (
  id: string,
  filename: string,
  content_type: string,
  size = 2048,
): Attachment => ({ id, filename, content_type, size });

// Markup only: node has no media elements, so `canPlayType` answers
// nothing and players fall back to the download row, as in a browser
// that cannot play the type.
describe("attachment list (#255)", () => {
  const html = renderToStaticMarkup(
    <AttachmentList
      attachments={[
        file("p1", "eins.png", "image/png"),
        file("p2", "zwei.jpg", "image/jpeg"),
        file("v", "clip.mov", "video/quicktime", 5 * 1024 * 1024),
        file("t", "notizen.txt", "text/plain"),
        file("big", "log.txt", "text/plain", 200 * 1024),
        file("z", "paket.zip", "application/zip"),
      ]}
    />,
  );

  it("puts several pictures in a grid", () => {
    expect(html).toContain("lr-attachment-grid");
    expect(html).toContain('alt="eins.png"');
    expect(html).toContain('alt="zwei.jpg"');
  });

  it("offers a video the browser cannot play as a download", () => {
    expect(html).not.toContain("<video");
    expect(html).toContain('href="/api/attachments/v?download=1"');
    expect(html).toContain("5,0 MB");
  });

  it("previews small text on demand and only downloads large text", () => {
    expect(html).toContain('<details class="lr-attachment-text"');
    expect(html).toContain("notizen.txt");
    expect(html).not.toMatch(/<details[^>]*>(?:(?!<\/details>).)*log\.txt/);
    expect(html).toContain('href="/api/attachments/big?download=1"');
  });

  it("saves other files, never opens them", () => {
    expect(html).toContain('href="/api/attachments/z?download=1"');
    expect(html).toContain('download="paket.zip"');
  });

  it("renders nothing without attachments", () => {
    expect(renderToStaticMarkup(<AttachmentList attachments={[]} />)).toBe("");
  });
});
