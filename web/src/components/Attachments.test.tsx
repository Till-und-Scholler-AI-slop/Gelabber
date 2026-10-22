import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Attachment } from "../messages/types.ts";
import { AttachmentImageFailed, AttachmentList } from "./Attachments.tsx";
import { reloadMedia } from "./mediaReload.ts";

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

describe("reloadMedia", () => {
  const media = (readyState: number, paused = false) => {
    let onMetadata = () => {};
    return {
      currentTime: 754,
      paused,
      readyState,
      load: vi.fn(),
      play: vi.fn(() => Promise.resolve()),
      addEventListener: (_: "loadedmetadata", listener: () => void) => {
        onMetadata = listener;
      },
      metadata: () => onMetadata(),
    };
  };

  it("fetches a fresh link once and returns to the same spot", () => {
    const el = media(4);
    expect(reloadMedia(el, -Infinity, 100_000)).toBe(true);
    expect(el.load).toHaveBeenCalledTimes(1);
    el.currentTime = 0;
    el.metadata();
    expect(el.currentTime).toBe(754);
    expect(el.play).toHaveBeenCalled();
    // A second failure right after: the file is unplayable after all.
    expect(reloadMedia(el, 100_000, 110_000)).toBe(false);
  });

  it("gives up on a file that never loaded", () => {
    const el = media(0, true);
    expect(reloadMedia(el, -Infinity, 100_000)).toBe(false);
    expect(el.load).not.toHaveBeenCalled();
  });
});

describe("picture that did not load (#348)", () => {
  it("says so and offers a retry and the file", () => {
    const html = renderToStaticMarkup(
      <AttachmentImageFailed
        attachment={file("p1", "IMG_9183.jpeg", "image/jpeg")}
        onRetry={() => undefined}
      />,
    );
    expect(html).toContain("lr-attachment-image is-failed");
    expect(html).toContain("IMG_9183.jpeg");
    expect(html).toContain("Bild konnte nicht geladen werden.");
    expect(html).toContain(">Erneut laden</button>");
    expect(html).toContain('href="/api/attachments/p1?download=1"');
    expect(html).toContain('download="IMG_9183.jpeg"');
  });

  it("has no download while the file is still on its way up", () => {
    const html = renderToStaticMarkup(
      <AttachmentImageFailed
        attachment={file("tmp:1", "neu.png", "image/png")}
        onRetry={() => undefined}
      />,
    );
    expect(html).toContain(">Erneut laden</button>");
    expect(html).not.toContain("Herunterladen");
  });
});
