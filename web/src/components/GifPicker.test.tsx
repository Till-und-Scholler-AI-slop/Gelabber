import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { Gif } from "../messages/types.ts";
import { GifImage } from "./GifPicker.tsx";

const gif: Gif = {
  provider: "klipy",
  slug: "hello-hi-662",
  title: "Hello",
  width: 498,
  height: 249,
  url: "https://static.klipy.com/ii/a/b.webp",
  still: "https://static.klipy.com/ii/a/b.jpg",
};

describe("gif image", () => {
  it("loads the animation or the still through the relay only", () => {
    const playing = renderToStaticMarkup(
      <GifImage gif={gif} playing alt="Hello" />,
    );
    expect(playing).toContain(
      `src="/api/gifs/media?url=${encodeURIComponent(gif.url)}"`,
    );
    const paused = renderToStaticMarkup(
      <GifImage gif={gif} playing={false} alt="Hello" />,
    );
    expect(paused).toContain(encodeURIComponent(gif.still!));
    expect(paused).not.toContain(encodeURIComponent(gif.url));
    expect(paused).not.toContain("static.klipy.com/");
  });

  it("shows a labelled placeholder instead of animating without a still", () => {
    const html = renderToStaticMarkup(
      <GifImage gif={{ ...gif, still: null }} playing={false} alt="Hello" />,
    );
    expect(html).not.toContain("<img");
    expect(html).toContain('aria-label="Hello"');
  });
});
