// Mentions (issue #241). On the wire a user is `<@user-id>`, so a rename does
// not break old messages; the composer shows `@Name` and swaps both ways.
// `@everyone` (all members) and `@here` (members online) stay as written.
// The server decides who is mentioned (api/src/messages/mentions.rs) and
// sends the result with the message as `mentions` / `mention_everyone`.
import type { Message } from "./types.ts";

export type MentionMember = { id: string; name: string };

const TOKEN =
  /<@([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})>/gi;
const KEYWORD = /(^|[^\p{L}\p{N}_@/-])@(everyone|here)(?![\p{L}\p{N}_@-])/gu;

/** Whether the message mentions `me`, as the server resolved it. */
export function mentionsMe(
  message: Pick<Message, "mentions" | "mention_everyone">,
  me: string | undefined,
): boolean {
  if (!me) return false;
  return Boolean(message.mention_everyone || message.mentions?.includes(me));
}

export type TextPart =
  | { kind: "text"; text: string }
  | { kind: "user"; id: string; name: string | null }
  | { kind: "everyone" | "here" };

/** Message text split into plain runs and mentions, for rendering. Code
 * (between backticks) is left alone, as the server does. */
export function splitMentions(
  content: string,
  members: readonly MentionMember[],
): TextPart[] {
  const names = new Map(members.map((m) => [m.id.toLowerCase(), m.name]));
  const parts: TextPart[] = [];
  const pushText = (text: string) => {
    if (!text) return;
    const last = parts[parts.length - 1];
    if (last?.kind === "text") last.text += text;
    else parts.push({ kind: "text", text });
  };
  const prose = (text: string) => {
    let at = 0;
    for (const match of text.matchAll(
      new RegExp(`${TOKEN.source}|${KEYWORD.source}`, "giu"),
    )) {
      const index = match.index ?? 0;
      // Escaped with a backslash (`\<@id>`, `\@here`): text, as the
      // server reads it. The keyword match starts one character early.
      const start = match[1] ? index : index + (match[2]?.length ?? 0);
      if (/(^|[^\\])(\\\\)*\\$/.test(text.slice(0, start))) continue;
      if (match[1]) {
        pushText(text.slice(at, index));
        const id = match[1].toLowerCase();
        parts.push({ kind: "user", id, name: names.get(id) ?? null });
      } else {
        // The keyword alternative also matched the character before it.
        const lead = match[2] ?? "";
        pushText(text.slice(at, index) + lead);
        parts.push({ kind: match[3] as "everyone" | "here" });
      }
      at = index + match[0].length;
    }
    pushText(text.slice(at));
  };
  for (const [index, segment] of codeSegments(content).entries()) {
    if (index % 2) pushText(segment);
    else prose(segment);
  }
  return parts;
}

/** Alternating prose and code: even entries are prose, odd ones code with
 * their backticks. An unclosed backtick run is prose. */
function codeSegments(content: string): string[] {
  const out: string[] = [];
  let prose = "";
  let rest = content;
  for (;;) {
    const start = rest.indexOf("`");
    if (start < 0) break;
    const fence = /^`+/.exec(rest.slice(start))![0];
    const end = rest.indexOf(fence, start + fence.length);
    if (end < 0) {
      prose += rest.slice(0, start + fence.length);
      rest = rest.slice(start + fence.length);
      continue;
    }
    out.push(prose + rest.slice(0, start));
    out.push(rest.slice(start, end + fence.length));
    prose = "";
    rest = rest.slice(end + fence.length);
  }
  out.push(prose + rest);
  return out;
}

const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** What the composer shows: `@Name` for every token of a known member. */
export function fromWire(
  content: string,
  members: readonly MentionMember[],
): string {
  const names = new Map(members.map((m) => [m.id.toLowerCase(), m.name]));
  return content.replace(TOKEN, (token, id: string) => {
    const name = names.get(id.toLowerCase());
    return name ? `@${name}` : token;
  });
}

/** Text for a preview or a notification: every token as `@Name`, one
 * of somebody no longer around as "@Unbekannt". */
export function readableMentions(
  content: string,
  members: readonly MentionMember[],
): string {
  return fromWire(content, members).replace(TOKEN, "@Unbekannt");
}

/** Who each `@Name` of `content` names, from its tokens: an edit keeps the
 * person it mentioned even where two members share the name. */
export function namesIn(
  content: string,
  members: readonly MentionMember[],
): Map<string, string> {
  const names = new Map(members.map((m) => [m.id.toLowerCase(), m.name]));
  const picked = new Map<string, string>();
  for (const [, id] of content.matchAll(TOKEN)) {
    const name = names.get(id.toLowerCase());
    if (name) picked.set(name, id);
  }
  return picked;
}

/** What is sent: every `@Name` of a member becomes their token. Longer names
 * first, so `@Ada Lovelace` is not taken for `@Ada`. Code stays as typed.
 * `picked` names who was chosen from the suggestions for a name; a name two
 * members share and nobody picked stays text rather than mention the wrong
 * one. A member called "everyone" or "here" never hides the keyword. */
export function toWire(
  text: string,
  members: readonly MentionMember[],
  picked: ReadonlyMap<string, string> = new Map(),
): string {
  const named = members
    .filter((m) => m.name.trim())
    .filter((m) => !/^(everyone|here)$/i.test(m.name))
    .sort((a, b) => b.name.length - a.name.length);
  if (!named.length || !text.includes("@")) return text;
  const pattern = new RegExp(
    `(^|[^\\p{L}\\p{N}_@-])@(${named.map((m) => escape(m.name)).join("|")})(?![\\p{L}\\p{N}_-])`,
    "gu",
  );
  const byName = new Map<string, string | null>();
  for (const member of named) {
    // Seen before: the name is ambiguous.
    byName.set(member.name, byName.has(member.name) ? null : member.id);
  }
  for (const [name, id] of picked) {
    if (named.some((m) => m.id === id && m.name === name)) byName.set(name, id);
  }
  return codeSegments(text)
    .map((segment, index) =>
      index % 2
        ? segment
        : segment.replace(pattern, (whole, lead: string, name: string) => {
            const id = byName.get(name);
            return id ? `${lead}<@${id}>` : whole;
          }),
    )
    .join("");
}

/** The `@…` being typed at the caret, if any: where it starts and what
 * follows the `@` so far. */
export function mentionAt(
  text: string,
  caret: number,
): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const match = /(^|[\s(])@([^\s@]{0,32})$/u.exec(before);
  if (!match) return null;
  return { start: caret - match[2].length - 1, query: match[2] };
}

export type Suggestion =
  { kind: "user"; id: string; name: string } | { kind: "everyone" | "here" };

/** Members whose name starts with the query come first, then those that
 * contain it; `@everyone` / `@here` where the conversation has them. */
export function mentionSuggestions(
  query: string,
  members: readonly MentionMember[],
  options: { me?: string; keywords: boolean; limit?: number },
): Suggestion[] {
  const q = query.toLocaleLowerCase();
  const others = members.filter((m) => m.id !== options.me);
  const starts = others.filter((m) => m.name.toLocaleLowerCase().startsWith(q));
  const contains = others.filter(
    (m) =>
      !m.name.toLocaleLowerCase().startsWith(q) &&
      m.name.toLocaleLowerCase().includes(q),
  );
  const users: Suggestion[] = [...starts, ...contains].map((m) => ({
    kind: "user",
    id: m.id,
    name: m.name,
  }));
  const keywords: Suggestion[] = options.keywords
    ? (["everyone", "here"] as const)
        .filter((word) => word.startsWith(q))
        .map((kind) => ({ kind }))
    : [];
  return [...users, ...keywords].slice(0, options.limit ?? 8);
}

/** The draft after picking `pick` for the `@…` at `start`, and the caret. */
export function insertMention(
  text: string,
  at: { start: number; query: string },
  pick: Suggestion,
): { text: string; caret: number } {
  const word = pick.kind === "user" ? pick.name : pick.kind;
  const end = at.start + 1 + at.query.length;
  const inserted = `@${word} `;
  const rest = text.slice(end).replace(/^ /, "");
  return {
    text: text.slice(0, at.start) + inserted + rest,
    caret: at.start + inserted.length,
  };
}
