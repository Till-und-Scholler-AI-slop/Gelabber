import { describe, expect, it } from "vitest";

import { asAttachmentList } from "./types.ts";

describe("asAttachmentList", () => {
  it("keeps arrays and maps empty objects to []", () => {
    expect(asAttachmentList([])).toEqual([]);
    expect(asAttachmentList({})).toEqual([]);
    expect(asAttachmentList(null)).toEqual([]);
    const files = [{ id: "a", filename: "a.png", content_type: "image/png", size: 1 }];
    expect(asAttachmentList(files)).toBe(files);
  });
});
