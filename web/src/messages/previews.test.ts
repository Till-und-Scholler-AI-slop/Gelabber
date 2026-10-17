import { describe, expect, it } from "vitest";

import { asPreviewList } from "./types.ts";

describe("link previews from the wire", () => {
  it("keeps whole previews and drops what the API would never send", () => {
    expect(
      asPreviewList([
        {
          url: "https://a.example/",
          title: "A",
          image: { url: "/api/previews/x/y", width: 10, height: 5 },
          youtube_id: "dQw4w9WgXcQ",
        },
        { url: "javascript:alert(1)", title: "x" },
        {
          url: "https://b.example/",
          image: { url: "https://b.example/i.png", width: 1, height: 1 },
          youtube_id: 'nope"><script>',
        },
        null,
      ]),
    ).toEqual([
      {
        url: "https://a.example/",
        title: "A",
        image: { url: "/api/previews/x/y", width: 10, height: 5 },
        youtube_id: "dQw4w9WgXcQ",
      },
      { url: "https://b.example/" },
    ]);
  });

  it("reads cjson's {} as no previews", () => {
    expect(asPreviewList({})).toEqual([]);
    expect(asPreviewList(undefined)).toEqual([]);
  });
});
