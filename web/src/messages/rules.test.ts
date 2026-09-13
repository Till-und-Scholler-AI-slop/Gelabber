import { describe, expect, it } from "vitest";

import {
  CONTENT_MAX,
  SIZE_MAX,
  inferContentType,
  normaliseContent,
  validateAttachment,
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
