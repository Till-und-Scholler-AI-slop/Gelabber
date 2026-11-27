import { describe, expect, it } from "vitest";
import { carriesFiles, droppedFiles, pastedFiles } from "./clipboard.ts";

const file = (name: string, type = "image/png") =>
  new File([new Uint8Array([1, 2, 3])], name, { type });

/** The very same files, in this order: a named file is passed on, not copied. */
function expectFiles(got: File[], want: File[]) {
  expect(got).toHaveLength(want.length);
  got.forEach((entry, index) => expect(entry).toBe(want[index]));
}

const text = { kind: "string", getAsFile: () => null };
const item = (entry: File | null) => ({
  kind: "file",
  getAsFile: () => entry,
});

describe("pasted files", () => {
  it("ignores text-only pastes", () => {
    expect(pastedFiles(null)).toEqual([]);
    expect(pastedFiles(undefined)).toEqual([]);
    expect(pastedFiles({})).toEqual([]);
    expect(pastedFiles({ files: [], items: [] })).toEqual([]);
    expect(pastedFiles({ files: [], items: [text, text] })).toEqual([]);
  });

  it("lets text win over the pictures office apps add", () => {
    expect(
      pastedFiles({
        getData: (format) => (format === "text/plain" ? "A1\tB1" : ""),
        files: [file("image.png"), file("image2.png")],
      }),
    ).toEqual([]);
  });

  it("takes the files when the text beside them is blank", () => {
    const picture = file("image.png");
    expectFiles(
      pastedFiles({
        getData: (format) => (format === "text/plain" ? " \n" : "<img>"),
        files: [picture],
      }),
      [picture],
    );
  });

  it("keeps every copied file with its name, in order", () => {
    const files = [
      file("katze.png"),
      file("notizen.pdf", "application/pdf"),
      file("hund.jpg", "image/jpeg"),
    ];
    expectFiles(pastedFiles({ files }), files);
  });

  it("takes screenshots from the clipboard items, between entries of other kinds", () => {
    const first = file("image.png");
    const second = file("image.jpg", "image/jpeg");
    expectFiles(
      pastedFiles({
        files: [],
        // An item the browser hands over without its file is left out.
        items: [text, item(first), item(null), text, item(second)],
      }),
      [first, second],
    );
  });

  it("does not take a copied file twice from files and items", () => {
    const copied = file("katze.png");
    expectFiles(pastedFiles({ files: [copied], items: [item(copied)] }), [
      copied,
    ]);
  });

  it("names each unnamed file by its type and leaves the named ones alone", () => {
    const named = file("katze.png");
    const pasted = pastedFiles(
      {
        files: [
          file(""),
          named,
          file(" ", "image/jpeg"),
          file("", "application/pdf"),
        ],
      },
      new Date("2026-10-09T12:34:56Z"),
    );
    expect(pasted.map((entry) => entry.name)).toEqual([
      "Bild-20261009-123456.png",
      "katze.png",
      "Bild-20261009-123456.jpg",
      "Bild-20261009-123456.bin",
    ]);
    expect(pasted[1]).toBe(named);
    expect(pasted.map((entry) => entry.type)).toEqual([
      "image/png",
      "image/png",
      "image/jpeg",
      "application/pdf",
    ]);
    expect(pasted.map((entry) => entry.size)).toEqual([3, 3, 3, 3]);
  });
});

describe("dropped files", () => {
  it("only reacts to drags that carry files", () => {
    expect(carriesFiles({ types: ["Files", "text/uri-list"] })).toBe(true);
    expect(carriesFiles({ types: ["text/plain", "text/html"] })).toBe(false);
    expect(carriesFiles(null)).toBe(false);
  });

  it("takes every file, in order, even when text comes along", () => {
    const files = [
      file("urlaub.png"),
      file("route.gpx", "application/gpx+xml"),
      file("strand.webp", "image/webp"),
    ];
    expectFiles(
      droppedFiles({ getData: () => "file:///home/kim/urlaub.png", files }),
      files,
    );
  });

  it("finds nothing in a drag of text or links", () => {
    expect(droppedFiles(null)).toEqual([]);
    expect(droppedFiles(undefined)).toEqual([]);
    expect(droppedFiles({})).toEqual([]);
    expect(droppedFiles({ files: [] })).toEqual([]);
    expect(
      droppedFiles({
        getData: () => "https://example.com/",
        files: [],
        items: [text, text],
      }),
    ).toEqual([]);
  });

  it("names each unnamed drop and leaves the named ones alone", () => {
    const named = file("urlaub.png");
    const dropped = droppedFiles(
      { files: [file(""), named, file("", "image/gif")] },
      new Date("2026-10-09T08:00:01Z"),
    );
    expect(dropped.map((entry) => entry.name)).toEqual([
      "Bild-20261009-080001.png",
      "urlaub.png",
      "Bild-20261009-080001.gif",
    ]);
    expect(dropped[1]).toBe(named);
  });
});
