import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MessageContent } from "../messages/MessageContent.tsx";
import { mentionRenderer } from "./mentionRenderer.tsx";
import type { MessagePing } from "./MessageMentions.tsx";

const ADA = "0190f7a2-0000-7000-8000-00000000000a";
const BOB = "0190f7a2-0000-7000-8000-00000000000b";
const MEMBERS = [
  { id: ADA, name: "Ada" },
  { id: BOB, name: "Bob" },
];

function render(content: string, ping: MessagePing, me = ADA) {
  return renderToStaticMarkup(
    <MessageContent
      content={content}
      renderMention={mentionRenderer(MEMBERS, me, content, ping)}
    />,
  );
}

describe("mentions in markdown", () => {
  it("names tokens and marks the own kept mention", () => {
    const html = render(`**hi** <@${BOB}> <@${ADA}>`, {
      everyone: false,
      mentions: [ADA, BOB],
    });
    expect(html).toContain("<strong>hi</strong>");
    expect(html).toContain('<span class="lr-mention ">@Bob</span>');
    expect(html).toContain('<span class="lr-mention is-me">@Ada</span>');
  });

  it("leaves unpinged keywords as text, code untouched", () => {
    const none = { everyone: false, mentions: [] };
    const html = render(`@everyone \`<@${BOB}>\``, none);
    expect(html).not.toContain("lr-mention");
    expect(html).toContain("@everyone");
    expect(html).toContain(`&lt;@${BOB}&gt;`);
  });

  it("highlights @here for whom it reached", () => {
    const reached = { everyone: false, mentions: [BOB] };
    expect(render("@here", reached, BOB)).toContain("is-me");
    expect(render("@here", reached, ADA)).toContain(
      'title="Alle, die gerade online waren"',
    );
  });
});
