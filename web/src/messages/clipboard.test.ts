import { describe, expect, it } from "vitest";
import { carriesFiles, droppedFile, pastedFile } from "./clipboard.ts";

const png = (name: string) =>
  new File([new Uint8Array([1, 2, 3])], name, { type: "image/png" });

describe("pasted files", () => {
  it("ignores text-only pastes", () => {
    expect(pastedFile(null)).toBeNull();
    expect(pastedFile({ files: [], items: [] })).toBeNull();
    expect(
      pastedFile({
        files: [],
        items: [{ kind: "string", getAsFile: () => null }],
      }),
    ).toBeNull();
  });

  it("lets text win over the picture office apps add", () => {
    expect(
      pastedFile({
        getData: (format) => (format === "text/plain" ? "A1\tB1" : ""),
        files: [png("image.png")],
      }),
    ).toBeNull();
  });

  it("keeps a copied file with its name", () => {
    const file = png("katze.png");
    expect(pastedFile({ files: [file] })).toBe(file);
  });

  it("takes a screenshot from the clipboard items", () => {
    const file = png("image.png");
    expect(
      pastedFile({
        files: [],
        items: [
          { kind: "string", getAsFile: () => null },
          { kind: "file", getAsFile: () => file },
        ],
      }),
    ).toBe(file);
  });

  it("names an unnamed image", () => {
    const named = pastedFile(
      { files: [png("")] },
      new Date("2026-10-09T12:34:56Z"),
    );
    expect(named?.name).toBe("Bild-20261009-123456.png");
    expect(named?.type).toBe("image/png");
    expect(named?.size).toBe(3);
  });
});

describe("dropped files", () => {
  it("only reacts to drags that carry files", () => {
    expect(carriesFiles({ types: ["Files", "text/uri-list"] })).toBe(true);
    expect(carriesFiles({ types: ["text/plain", "text/html"] })).toBe(false);
    expect(carriesFiles(null)).toBe(false);
  });

  it("takes the first file even when text comes along", () => {
    const file = png("urlaub.png");
    expect(
      droppedFile({
        getData: () => "file:///home/kim/urlaub.png",
        files: [file, png("zwei.png")],
      }),
    ).toBe(file);
    expect(droppedFile({ files: [] })).toBeNull();
  });

  it("names an unnamed drop", () => {
    expect(
      droppedFile({ files: [png("")] }, new Date("2026-10-09T08:00:01Z"))?.name,
    ).toBe("Bild-20261009-080001.png");
  });
});
