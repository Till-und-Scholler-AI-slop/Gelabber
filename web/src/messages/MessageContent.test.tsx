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

  it("opens mail links in place, not in an empty tab", () => {
    expect(html("<mailto:a@example.com>")).toContain(
      '<a href="mailto:a@example.com" class="lr-md-link">',
    );
  });

  it("keeps links in a hidden spoiler out of reach until revealed", () => {
    const out = html("||see https://example.com||");
    expect(out).toContain(
      '<span class="lr-md-spoiler" role="button" tabindex="0" aria-label="Spoiler anzeigen"><span aria-hidden="true" inert="">',
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
    expect(out).toMatch(/<button[^>]*>.*<span>Kopieren<\/span>.*<\/button>/);
    expect(out).not.toContain("<a ");
  });

  it("uses the mention renderer and falls back to the raw token", () => {
    const id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    expect(html(`hi <@${id}>`)).toContain(`&lt;@${id}&gt;`);
    expect(
      renderToStaticMarkup(
        <MessageContent
          content={`hi <@${id}>`}
          renderMention={(part) =>
            part.type === "mention" ? <b>@{part.userId.slice(0, 4)}</b> : null
          }
        />,
      ),
    ).toContain("<b>@0f8f</b>");
    expect(html("an @everyone und @here")).toContain(
      "an <span>@everyone</span> und <span>@here</span>",
    );
    expect(
      renderToStaticMarkup(
        <MessageContent
          content="@here"
          renderMention={(part) => <b>{part.type}</b>}
        />,
      ),
    ).toContain("<b>here</b>");
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
      '<span class="lr-md-spoiler" role="button" tabindex="0" aria-label="Spoiler anzeigen"><span aria-hidden="true" inert="">secret</span></span>',
    );
  });
});
