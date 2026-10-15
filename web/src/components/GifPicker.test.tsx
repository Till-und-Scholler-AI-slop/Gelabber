import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ApiError } from "../api/client.ts";
import type { Gif } from "../messages/types.ts";
import { gifFeedStatus } from "../gifs/api.ts";
import { GifImage, GifMessageButton } from "./GifPicker.tsx";

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

describe("gif message", () => {
  it("plays unless reduced motion is on, then shows the still", () => {
    const playing = renderToStaticMarkup(
      <GifMessageButton gif={gif} reduced={false} />,
    );
    expect(playing).toContain('aria-pressed="true"');
    expect(playing).toContain(encodeURIComponent(gif.url));
    expect(playing).not.toContain("lr-message-gif-badge");

    const still = renderToStaticMarkup(<GifMessageButton gif={gif} reduced />);
    expect(still).toContain('aria-pressed="false"');
    expect(still).toContain(encodeURIComponent(gif.still!));
    expect(still).not.toContain(encodeURIComponent(gif.url));
    expect(still).toContain("lr-message-gif-badge");
  });

  it("has a neutral toggle name, the pressed state tells playback", () => {
    const html = renderToStaticMarkup(
      <GifMessageButton gif={gif} reduced={false} />,
    );
    expect(html).toContain('aria-label="GIF: Hello, Animation"');
    expect(html).not.toContain("abspielen");
  });
});

describe("gif picker status", () => {
  const base = { pending: false, error: null, count: 0, query: "" };

  it("says when the GIF budget allows the next request", () => {
    const limited = new ApiError("rate_limited", 429, "x", {}, 12);
    expect(gifFeedStatus({ ...base, error: limited })).toBe(
      "Zu viele GIF-Anfragen. Bitte in 12 s noch einmal versuchen.",
    );
    expect(
      gifFeedStatus({
        ...base,
        error: new ApiError("rate_limited", 429, "x"),
        count: 24,
      }),
    ).toContain("Zu viele Anfragen");
  });

  it("names a provider outage and other failures apart", () => {
    expect(
      gifFeedStatus({
        ...base,
        error: new ApiError("upstream_unavailable", 502, "x"),
      }),
    ).toContain("GIF-Dienst");
    expect(gifFeedStatus({ ...base, error: new Error("boom") })).toBe(
      "GIFs konnten nicht geladen werden.",
    );
    expect(
      gifFeedStatus({ ...base, error: new Error("boom"), count: 24 }),
    ).toBe("Weitere GIFs konnten nicht geladen werden.");
  });

  it("counts results otherwise", () => {
    expect(gifFeedStatus({ ...base, pending: true })).toBe(
      "GIFs werden geladen …",
    );
    expect(gifFeedStatus(base)).toBe("Keine GIFs gefunden.");
    expect(gifFeedStatus({ ...base, count: 3, query: "hallo" })).toBe(
      "3 GIFs zu „hallo“",
    );
    expect(gifFeedStatus({ ...base, count: 3 })).toBe("3 angesagte GIFs");
  });
});
