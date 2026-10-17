import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { type MessagePing, MessageText } from "./MessageMentions.tsx";
import { MentionContext } from "./mentionContext.ts";

const ADA = "0190f7a2-0000-7000-8000-00000000000a";
const BOB = "0190f7a2-0000-7000-8000-00000000000b";

function render(
  content: string,
  me = ADA,
  ping: MessagePing | null = { everyone: true, mentions: [] },
) {
  return renderToStaticMarkup(
    <MentionContext.Provider
      value={{
        members: [
          { id: ADA, name: "Ada" },
          { id: BOB, name: "Bob" },
        ],
        me,
        keywords: true,
      }}
    >
      <MessageText content={content} ping={ping ?? undefined} />
    </MentionContext.Provider>,
  );
}

describe("message text", () => {
  it("shows names for tokens and marks the own mention", () => {
    const html = render(`hi <@${BOB}> and <@${ADA}>`);
    expect(html).toBe(
      'hi <span class="lr-mention ">@Bob</span> and <span class="lr-mention is-me">@Ada</span>',
    );
  });

  it("marks @everyone for everyone, names a stranger Unbekannt", () => {
    expect(render("@everyone look")).toContain(
      '<span class="lr-mention is-me">@everyone</span> look',
    );
    expect(render("<@0190f7a2-0000-7000-8000-0000000000ff>", BOB)).toContain(
      "@Unbekannt",
    );
  });

  it("leaves @everyone and @here as text when they pinged nobody", () => {
    const none = { everyone: false, mentions: [] };
    expect(render("@everyone look", ADA, none)).toBe("@everyone look");
    expect(
      render(`@here <@${BOB}>`, ADA, { everyone: false, mentions: [BOB] }),
    ).toBe('@here <span class="lr-mention ">@Bob</span>');
    expect(render("@everyone", ADA, null)).toBe("@everyone");
  });

  it("marks @here as own only for those it reached", () => {
    const reached = { everyone: false, mentions: [BOB] };
    expect(render("@here", ADA, reached)).toBe(
      '<span class="lr-mention " title="Alle, die gerade online waren">@here</span>',
    );
    expect(render("@here", BOB, reached)).toContain("is-me");
  });

  it("leaves code and plain text alone, escaped", () => {
    expect(render(`\`<@${BOB}>\` <b>`)).toBe(`\`&lt;@${BOB}&gt;\` &lt;b&gt;`);
  });
});
