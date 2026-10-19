// Mentions inside markdown text (`MessageContent`, issue #241): the same
// highlighting as `MessageText`, as a renderer for its mention parts.
import { useMemo, type ReactNode } from "react";

import type {
  MentionPart,
  MentionRenderer,
} from "../messages/MessageContent.tsx";
import { splitMentions, type MentionMember } from "../messages/mentions.ts";
import type { MessagePing } from "./MessageMentions.tsx";
import { useMentions } from "./mentionContext.ts";

/** What a stored message pinged, for `mentionRenderer`. */
export function messagePing(message: {
  mentions?: string[];
  mention_everyone?: boolean;
}): MessagePing {
  return {
    everyone: message.mention_everyone === true,
    mentions: message.mentions ?? [],
  };
}

/** Draws the mentions of markdown text (`MessageContent`) the way
 * `MessageText` does: names for tokens, `@everyone` / `@here` highlighted
 * only where `ping` shows they reached someone. */
export function mentionRenderer(
  members: readonly MentionMember[],
  me: string | undefined,
  content: string,
  ping?: MessagePing,
): MentionRenderer {
  const names = new Map(members.map((m) => [m.id.toLowerCase(), m.name]));
  const named = new Set(
    splitMentions(content, members).flatMap((part) =>
      part.kind === "user" ? [part.id] : [],
    ),
  );
  // `@here` is stored as the users it reached, beyond the named ones.
  const hereReached =
    !!ping &&
    (ping.everyone || ping.mentions.some((id) => !named.has(id.toLowerCase())));
  const mentionsMe = !!ping?.mentions.some((id) => id === me);
  return (part: MentionPart): ReactNode => {
    if (part.type === "everyone" && !ping?.everyone) return "@everyone";
    if (part.type === "here" && !hereReached) return "@here";
    // A user token is "me" only where the server kept it as a mention.
    const own =
      part.type === "mention"
        ? part.userId === me && (!ping || ping.everyone || mentionsMe)
        : !!ping?.everyone || mentionsMe;
    const label =
      part.type === "mention"
        ? `@${names.get(part.userId) ?? "Unbekannt"}`
        : `@${part.type}`;
    return (
      <span
        className={`lr-mention ${own ? "is-me" : ""}`}
        title={
          part.type === "here" ? "Alle, die gerade online waren" : undefined
        }
      >
        {label}
      </span>
    );
  };
}

/** `mentionRenderer` for one stored message of the pane. */
export function useMentionRenderer(message: {
  content: string;
  mentions?: string[];
  mention_everyone?: boolean;
}): MentionRenderer {
  const { members, me } = useMentions();
  const { content, mentions, mention_everyone } = message;
  return useMemo(
    () =>
      mentionRenderer(members, me, content, {
        everyone: mention_everyone === true,
        mentions: mentions ?? [],
      }),
    [members, me, content, mentions, mention_everyone],
  );
}
