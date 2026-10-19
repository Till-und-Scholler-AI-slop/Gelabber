// Mentions in the message pane (issue #241): the text of a message with its
// mentions picked out, and the `@…` suggestions of the composer.
import { useMemo } from "react";

import { splitMentions } from "../messages/mentions.ts";
import { useMentions } from "./mentionContext.ts";

/** What the server stored for a message: `@everyone` / `@here` without
 * the right (or over the limit) went out as plain text. */
export interface MessagePing {
  everyone: boolean;
  mentions: readonly string[];
}

/** Message text, mentions as highlighted names. `@everyone` and `@here`
 * are highlighted only where `ping` shows they reached someone. */
export function MessageText({
  content,
  ping,
}: {
  content: string;
  ping?: MessagePing;
}) {
  const { members, me } = useMentions();
  const parts = useMemo(
    () => splitMentions(content, members),
    [content, members],
  );
  const named = new Set(
    parts.flatMap((part) =>
      part.kind === "user" ? [part.id.toLowerCase()] : [],
    ),
  );
  // `@here` is stored as the users it reached, beyond the named ones.
  const hereReached =
    !!ping &&
    (ping.everyone || ping.mentions.some((id) => !named.has(id.toLowerCase())));
  const mentionsMe = !!ping?.mentions.some((id) => id === me);
  return parts.map((part, index) => {
    if (part.kind === "text") return part.text;
    if (part.kind === "everyone" && !ping?.everyone) return "@everyone";
    if (part.kind === "here" && !hereReached) return "@here";
    // A user token is "me" only where the server kept it as a mention.
    const own =
      part.kind === "user"
        ? part.id === me && (!ping || ping.everyone || mentionsMe)
        : !!ping?.everyone || mentionsMe;
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
