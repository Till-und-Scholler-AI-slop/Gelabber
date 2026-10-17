import { describe, expect, it } from "vitest";

import {
  CONTENT_MAX,
  SIZE_MAX,
  inferContentType,
  normaliseContent,
  validateAttachment,
  validateAttachmentSet,
  validateContent,
} from "./rules.ts";

describe("message content (mirror of api/src/messages/validate.rs)", () => {
  it("trims and normalises newlines", () => {
    expect(normaliseContent("  hi  ")).toBe("hi");
    expect(normaliseContent("win\r\nline")).toBe("win\nline");
    expect(validateContent("  hallo  ")).toBeNull();
    expect(validateContent("ok\nline\t2")).toBeNull();
  });

  it("rejects empty, too long, and control characters", () => {
    expect(validateContent("   ")).toBe("required");
    expect(validateContent("x".repeat(CONTENT_MAX + 1))).toBe("too_long");
    expect(validateContent("a\u0000b")).toBe("invalid");
  });
});

describe("attachment limits (mirror of api/src/attachments/validate.rs)", () => {
  it("infers type from extension when the browser leaves type empty", () => {
    expect(inferContentType({ name: "cat.PNG", type: "" })).toBe("image/png");
    expect(inferContentType({ name: "a.jpg", type: "image/jpeg" })).toBe(
      "image/jpeg",
    );
  });

  it("rejects type and size the API would reject", () => {
    expect(
      validateAttachment({
        name: "x.exe",
        type: "application/x-msdownload",
        size: 10,
      }),
    ).toEqual({ field: "content_type", code: "invalid" });
    expect(
      validateAttachment({
        name: "big.png",
        type: "image/png",
        size: SIZE_MAX + 1,
      }),
    ).toEqual({ field: "size", code: "too_long" });
    expect(
      validateAttachment({ name: "ok.png", type: "image/png", size: 12 }),
    ).toBeNull();
  });
});

describe("attachment types and limits (#255)", () => {
  it("declares source files as text and maps browser aliases", () => {
    expect(inferContentType({ name: "main.rs", type: "" })).toBe("text/plain");
    expect(inferContentType({ name: "app.py", type: "text/x-python" })).toBe(
      "text/plain",
    );
    expect(inferContentType({ name: "a.js", type: "text/javascript" })).toBe(
      "text/plain",
    );
    expect(inferContentType({ name: "song.m4a", type: "audio/x-m4a" })).toBe(
      "audio/mp4",
    );
    expect(inferContentType({ name: "clip.mov", type: "" })).toBe(
      "video/quicktime",
    );
    expect(
      inferContentType({ name: "backup.tar.gz", type: "application/x-gzip" }),
    ).toBe("application/gzip");
  });

  it("never accepts HTML or SVG", () => {
    for (const file of [
      { name: "page.html", type: "text/html", size: 4 },
      { name: "logo.svg", type: "image/svg+xml", size: 4 },
      { name: "page.htm", type: "", size: 4 },
    ])
      expect(validateAttachment(file)?.field).toBe("content_type");
  });

  it("limits a message to ten files and 100 MB together", () => {
    const small = { size: 1 };
    expect(validateAttachmentSet(Array(10).fill(small))).toBeNull();
    expect(validateAttachmentSet(Array(11).fill(small))?.code).toBe("too_long");
    expect(
      validateAttachmentSet(Array(5).fill({ size: 21 * 1024 * 1024 }))?.code,
    ).toBe("too_large");
  });
});
