import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Attachment } from "../messages/types.ts";
import {
  AttachmentImageFailed,
  AttachmentList,
  DownloadLink,
} from "./Attachments.tsx";
import { rememberImageSize } from "./imageBox.ts";
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

// Markup only; scripts/smoke-chat-scroll.mjs checks the boxes in a browser.
describe("picture box", () => {
  it("is the picture's own shape once its size is known", () => {
    const shot = file("s1", "zeile.png", "image/png");
    expect(
      renderToStaticMarkup(<AttachmentList attachments={[shot]} />),
    ).toContain('class="lr-attachment-image is-unsized"');
    rememberImageSize(shot, { width: 972, height: 114 });
    const html = renderToStaticMarkup(<AttachmentList attachments={[shot]} />);
    expect(html).toContain('class="lr-attachment-image is-sized"');
    // Wider than 3:1: the box keeps 3:1 and crops.
    expect(html).toContain("width:min(100%, 342px);aspect-ratio:342 / 114");
  });

  it("takes the size and placeholder colour the API sends (#574)", () => {
    const photo: Attachment = {
      ...file("api1", "IMG_2031.jpg", "image/jpeg", 6 * 1024 * 1024),
      width: 3000,
      height: 4000,
      placeholder: "#7a6b5c",
      placeholder_version: 1,
      thumbnail: true,
    };
    const html = renderToStaticMarkup(<AttachmentList attachments={[photo]} />);
    expect(html).toContain("width:min(100%, 300px);aspect-ratio:300 / 400");
    expect(html).toContain("background-color:#7a6b5c");
    // The thumbnail, not the 6 MB original, at both densities.
    expect(html).toContain('src="/api/attachments/api1?size=400"');
    expect(html).toContain(
      'srcSet="/api/attachments/api1?size=400 300w, /api/attachments/api1?size=800 600w"',
    );
    expect(html).toContain('sizes="300px"');
  });

  it("picks the thumbnail by the width the picture is drawn at (#574)", () => {
    const pic = (id: string, width: number, height: number): Attachment => ({
      ...file(id, `${id}.png`, "image/png"),
      width,
      height,
      thumbnail: true,
    });
    // A long screenshot cropped to 134x400: the 400 thumbnail is 72 px
    // wide, too narrow for the box at any density.
    const long = renderToStaticMarkup(
      <AttachmentList attachments={[pic("s1", 1080, 6000)]} />,
    );
    expect(long).toContain("size=400 72w");
    expect(long).toContain("size=800 144w");
    // A 4:3 photo in a mosaic tile covers it, so it is drawn wider.
    const tiles = renderToStaticMarkup(
      <AttachmentList
        attachments={[pic("m1", 4000, 3000), pic("m2", 10, 10)]}
      />,
    );
    expect(tiles).toContain('sizes="(max-width: 600px) 67vw, 267px"');
  });

  it("shows the original until there is a thumbnail", () => {
    const photo: Attachment = {
      ...file("api2", "a.jpg", "image/jpeg"),
      width: 10,
      height: 10,
    };
    const html = renderToStaticMarkup(<AttachmentList attachments={[photo]} />);
    expect(html).toContain('src="/api/attachments/api2"');
    expect(html).not.toContain("srcSet");
  });

  it("plays small animations inline, large ones in the viewer", () => {
    const gif = (id: string, size: number): Attachment => ({
      ...file(id, "x.gif", "image/gif", size),
      width: 100,
      height: 100,
      animated: true,
      thumbnail: true,
    });
    const small = renderToStaticMarkup(
      <AttachmentList attachments={[gif("g1", 1024 * 1024)]} />,
    );
    expect(small).toContain('src="/api/attachments/g1"');
    expect(small).not.toContain("lr-attachment-badge");
    const large = renderToStaticMarkup(
      <AttachmentList attachments={[gif("g2", 8 * 1024 * 1024)]} />,
    );
    expect(large).toContain('src="/api/attachments/g2?size=400"');
    expect(large).toContain('<span class="lr-attachment-badge">GIF</span>');
  });

  it("lays out several pictures as a mosaic by count", () => {
    const shots = (n: number) =>
      Array.from({ length: n }, (_, i) =>
        file(`m${n}-${i}`, "a.png", "image/png"),
      );
    const names: [number, string][] = [
      [2, "is-two"],
      [3, "is-three"],
      [4, "is-four"],
      [5, "is-five"],
      [6, "is-six"],
      [10, "is-six"],
    ];
    for (const [count, name] of names) {
      const html = renderToStaticMarkup(
        <AttachmentList attachments={shots(count)} />,
      );
      expect(html).toContain(`lr-attachment-grid ${name}`);
      expect(html.match(/lr-attachment-image is-tile/g)?.length).toBe(
        Math.min(count, 6),
      );
      expect(html).not.toContain("is-sized");
    }
    const ten = renderToStaticMarkup(
      <AttachmentList attachments={shots(10)} />,
    );
    expect(ten).toContain('aria-label="4 weitere Bilder">+4</span>');
  });

  it("shows upload progress in percent on the picture", () => {
    const pending: Attachment = {
      ...file("tmp:1", "a.png", "image/png"),
      preview_url: "blob:x",
      upload: "uploading",
      progress: 0.42,
    };
    const html = renderToStaticMarkup(
      <AttachmentList attachments={[pending]} />,
    );
    expect(html).toContain("Wird hochgeladen … 42 %");
    expect(html).toContain("width:42%");
  });
});

// Markup only, before any click; scripts/smoke-chat-scroll.mjs clicks.
describe("download link (#391)", () => {
  it("saves the file and takes clicks until one started it", () => {
    expect(
      renderToStaticMarkup(
        <DownloadLink href="/api/attachments/d?download=1" filename="a.pdf" />,
      ),
    ).toBe(
      '<a href="/api/attachments/d?download=1" download="a.pdf">Herunterladen</a>',
    );
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
