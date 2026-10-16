import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MessageContent } from "./MessageContent.tsx";

const html = (content: string, interactive = true) =>
  renderToStaticMarkup(
    <MessageContent content={content} interactive={interactive} />,
  );

describe("message content", () => {
  it("opens links in a new tab without opener or referrer", () => {
    expect(html("see https://example.com/x")).toContain(
      '<a href="https://example.com/x" target="_blank" rel="noopener noreferrer nofollow" class="lr-md-link">https://example.com/x</a>',
    );
  });

  it("escapes HTML instead of rendering it", () => {
    const out = html('<img src=x onerror="alert(1)"> **<b>x</b>**');
    expect(out).not.toContain("<img");
    expect(out).not.toContain("<b>");
    expect(out).toContain("&lt;img");
    expect(out).toContain("<strong>&lt;b&gt;x&lt;/b&gt;</strong>");
  });

  it("renders code blocks with a copy button and keeps their text as is", () => {
    const out = html("```js\n**x** https://a.example\n```");
    expect(out).toContain(
      '<code data-lang="js">**x** https://a.example</code>',
    );
    expect(out).toContain(">Kopieren</button>");
    expect(out).not.toContain("<a ");
  });

  it("uses the mention renderer and falls back to the raw token", () => {
    const id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    expect(html(`hi <@${id}>`)).toContain(`&lt;@${id}&gt;`);
    expect(
      renderToStaticMarkup(
        <MessageContent
          content={`hi <@${id}>`}
          renderMention={(userId) => <b>@{userId.slice(0, 4)}</b>}
        />,
      ),
    ).toContain("<b>@0f8f</b>");
  });

  it("has no links or buttons when it sits inside a button", () => {
    const out = html("https://a.example ||x|| ```\ncode\n```", false);
    expect(out).not.toContain("<a ");
    expect(out).not.toContain("<button");
    expect(out).not.toContain("<div");
    expect(out).not.toContain('role="button"');
  });

  it("hides spoilers until they are clicked", () => {
    expect(html("||secret||")).toContain(
      '<span class="lr-md-spoiler" role="button" tabindex="0" aria-label="Spoiler anzeigen"><span aria-hidden="true">secret</span></span>',
    );
  });
});
