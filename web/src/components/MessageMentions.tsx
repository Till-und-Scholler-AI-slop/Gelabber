// Mentions in the message pane (issue #241): the text of a message with its
// mentions picked out, and the `@…` suggestions of the composer.
import { useMemo } from "react";

import { splitMentions } from "../messages/mentions.ts";
import { useMentions } from "./mentionContext.ts";

/** Message text, mentions as highlighted names. */
export function MessageText({ content }: { content: string }) {
  const { members, me } = useMentions();
  const parts = useMemo(
    () => splitMentions(content, members),
    [content, members],
  );
  return parts.map((part, index) => {
    if (part.kind === "text") return part.text;
    const own = part.kind !== "user" || part.id === me;
    const label =
      part.kind === "user" ? `@${part.name ?? "Unbekannt"}` : `@${part.kind}`;
    return (
      <span
        key={index}
        className={`lr-mention ${own ? "is-me" : ""}`}
        title={
          part.kind === "here" ? "Alle, die gerade online waren" : undefined
        }
      >
        {label}
      </span>
    );
  });
}
