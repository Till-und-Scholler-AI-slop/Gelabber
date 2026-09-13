import { describe, expect, it } from "vitest";

import { CONTENT_MAX, normaliseContent, validateContent } from "./rules.ts";

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
