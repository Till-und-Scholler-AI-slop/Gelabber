// Message markdown (#250): the Discord-flavoured subset, parsed on the client
// into a small tree. The server stores and validates raw text only. Nothing
// here produces HTML from the text, so a tag stays text and only http(s) and
// mailto links become links.
//
// Blocks: ```lang fenced code```, `> quote` lines and `>>> ` (rest of the
// message). Inline: **bold**, *italic* / _italic_, __underline__, ~~strike~~,
// ||spoiler||, `code`, bare http(s) URLs, <url> (a link without preview),
// <@user-id> mentions and \-escapes. No headings, lists, tables or images.

export type Inline =
  | { type: "text"; text: string }
  | {
      type: "strong" | "em" | "underline" | "strike" | "spoiler";
      children: Inline[];
    }
  | { type: "code"; text: string }
  | { type: "link"; href: string; text: string; suppressed: boolean }
  | { type: "mention"; userId: string };

export type Block =
  | { type: "paragraph"; children: Inline[] }
  | { type: "quote"; children: Inline[] }
  | { type: "codeblock"; lang: string | null; text: string };

const FENCE = "```";
/** Nested formatting deeper than this stays literal text. */
const MAX_DEPTH = 8;
const LANG = /^([A-Za-z0-9_+#.-]{1,32})\n/;
const URL_START = /^https?:\/\/[^\s<>"]+/i;
const ANGLE_LINK = /^<((?:https?:\/\/|mailto:)[^\s<>"]+)>/i;
const MENTION =
  /^<@!?([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})>/i;
const WORD = /[\p{L}\p{N}_]/u;
const ALNUM = /[\p{L}\p{N}]/u;
const SPACE = /\s/;

type Mark = {
  mark: string;
  type: "strong" | "em" | "underline" | "strike" | "spoiler";
};
// Longer marks first, so `**` is bold before it is tried as two italics.
const MARKS: Mark[] = [
  { mark: "**", type: "strong" },
  { mark: "__", type: "underline" },
  { mark: "~~", type: "strike" },
  { mark: "||", type: "spoiler" },
  { mark: "*", type: "em" },
  { mark: "_", type: "em" },
];

/** The message as blocks; an empty message has none. */
export function parseMessage(src: string): Block[] {
  const blocks: Block[] = [];
  let text = "";
  let i = 0;
  while (i < src.length) {
    if (src[i] === "\\" && i + 1 < src.length) {
      // Kept for the inline pass, which drops the backslash.
      text += src.slice(i, i + 2);
      i += 2;
      continue;
    }
    if (src.startsWith(FENCE, i)) {
      const close = src.indexOf(FENCE, i + FENCE.length);
      const code = close === -1 ? null : codeBlock(src.slice(i + 3, close));
      if (code) {
        pushText(blocks, text.endsWith("\n") ? text.slice(0, -1) : text);
        blocks.push(code);
        text = "";
        i = close + FENCE.length;
        if (src[i] === "\n") i++;
        continue;
      }
    }
    text += src[i];
    i++;
  }
  pushText(blocks, text);
  return blocks;
}

function codeBlock(body: string): Block | null {
  const lang = LANG.exec(body);
  let text = lang ? body.slice(lang[0].length) : body;
  if (text.startsWith("\n")) text = text.slice(1);
  if (text.endsWith("\n")) text = text.slice(0, -1);
  if (text.trim() === "") return null;
  return { type: "codeblock", lang: lang ? lang[1].toLowerCase() : null, text };
}

/** Plain text between code blocks: paragraphs and quotes, line by line. */
function pushText(blocks: Block[], text: string) {
  if (text === "") return;
  const lines = text.split("\n");
  let plain: string[] = [];
  let quoted: string[] = [];
  const flushPlain = () => {
    if (plain.length) {
      const joined = plain.join("\n");
      if (joined !== "")
        blocks.push({ type: "paragraph", children: parseInline(joined) });
    }
    plain = [];
  };
  const flushQuote = () => {
    if (quoted.length)
      blocks.push({ type: "quote", children: parseInline(quoted.join("\n")) });
    quoted = [];
  };
  for (let n = 0; n < lines.length; n++) {
    const line = lines[n];
    if (line.startsWith(">>> ")) {
      flushPlain();
      quoted.push(line.slice(4), ...lines.slice(n + 1));
      break;
    }
    if (line.startsWith("> ")) {
      flushPlain();
      quoted.push(line.slice(2));
    } else {
      flushQuote();
      plain.push(line);
    }
  }
  flushPlain();
  flushQuote();
}

/** Inline formatting of one paragraph or quote. */
export function parseInline(src: string, depth = 0): Inline[] {
  const out: Inline[] = [];
  let text = "";
  const push = (node: Inline) => {
    if (text) out.push({ type: "text", text });
    text = "";
    out.push(node);
  };
  let i = 0;
  outer: while (i < src.length) {
    const c = src[i];
    if (
      c === "\\" &&
      i + 1 < src.length &&
      !ALNUM.test(src[i + 1]) &&
      !SPACE.test(src[i + 1])
    ) {
      text += src[i + 1];
      i += 2;
      continue;
    }
    if (c === "`") {
      const run = runLength(src, i, "`");
      const end = codeSpanEnd(src, i, run);
      if (end === -1) {
        text += src.slice(i, i + run);
        i += run;
        continue;
      }
      let code = src.slice(i + run, end - run);
      if (
        code.length > 2 &&
        code.startsWith(" ") &&
        code.endsWith(" ") &&
        code.trim()
      )
        code = code.slice(1, -1);
      push({ type: "code", text: code });
      i = end;
      continue;
    }
    if (c === "<") {
      const mention = MENTION.exec(src.slice(i));
      if (mention) {
        push({ type: "mention", userId: mention[1].toLowerCase() });
        i += mention[0].length;
        continue;
      }
      const angle = ANGLE_LINK.exec(src.slice(i));
      const href = angle && safeHref(angle[1]);
      if (angle && href) {
        push({ type: "link", href, text: angle[1], suppressed: true });
        i += angle[0].length;
        continue;
      }
    }
    const url = bareUrlAt(src, i);
    if (url) {
      push({ type: "link", href: url.href, text: url.text, suppressed: false });
      i += url.text.length;
      continue;
    }
    if (depth < MAX_DEPTH) {
      for (const { mark, type } of MARKS) {
        if (!src.startsWith(mark, i) || !opens(src, i, mark)) continue;
        const from = i + mark.length;
        const close = findCloser(src, from, mark);
        if (close <= from) continue;
        push({
          type,
          children: parseInline(src.slice(from, close), depth + 1),
        });
        i = close + mark.length;
        continue outer;
      }
    }
    text += c;
    i++;
  }
  if (text) out.push({ type: "text", text });
  return out;
}

function opens(src: string, i: number, mark: string): boolean {
  const next = src[i + mark.length];
  if (next === undefined) return false;
  if (mark === "*") return !SPACE.test(next);
  if (mark === "_")
    return !SPACE.test(next) && (i === 0 || !WORD.test(src[i - 1]));
  return true;
}

/** Where `mark` closes, scanning past escapes, code, links and mentions. */
function findCloser(src: string, from: number, mark: string): number {
  const single = mark.length === 1;
  let j = from;
  while (j < src.length) {
    const c = src[j];
    if (c === "\\") {
      j += 2;
      continue;
    }
    if (c === "`") {
      const run = runLength(src, j, "`");
      const end = codeSpanEnd(src, j, run);
      j = end === -1 ? j + run : end;
      continue;
    }
    if (c === "<") {
      const tag = MENTION.exec(src.slice(j)) ?? ANGLE_LINK.exec(src.slice(j));
      if (tag) {
        j += tag[0].length;
        continue;
      }
    }
    const url = bareUrlAt(src, j);
    if (url) {
      j += url.text.length;
      continue;
    }
    if (src.startsWith(mark, j)) {
      if (single) {
        // A double mark inside an italic is a nested bold or underline.
        if (src[j + 1] === mark) {
          j += runLength(src, j, mark);
          continue;
        }
        if (mark === "*" && SPACE.test(src[j - 1])) {
          j++;
          continue;
        }
        if (mark === "_" && j + 1 < src.length && WORD.test(src[j + 1])) {
          j++;
          continue;
        }
        return j;
      }
      if (mark === "**" || mark === "__") {
        // `***a***`: the outer bold closes on the run's last two marks.
        return j + runLength(src, j, mark[0]) - 2;
      }
      return j;
    }
    j++;
  }
  return -1;
}

function runLength(src: string, i: number, ch: string): number {
  let n = 0;
  while (src[i + n] === ch) n++;
  return n;
}

/** The end of a code span opened by `run` backticks at `i`, or -1. */
function codeSpanEnd(src: string, i: number, run: number): number {
  let j = i + run;
  while (j < src.length) {
    const next = src.indexOf("`", j);
    if (next === -1) return -1;
    const len = runLength(src, next, "`");
    if (len === run) return next + len;
    j = next + len;
  }
  return -1;
}

/** A bare http(s) URL starting at `i`, trailing punctuation left out. */
function bareUrlAt(
  src: string,
  i: number,
): { text: string; href: string } | null {
  const c = src[i];
  if (c !== "h" && c !== "H") return null;
  if (i > 0 && ALNUM.test(src[i - 1])) return null;
  const match = URL_START.exec(src.slice(i, i + 2048));
  if (!match) return null;
  const text = trimUrl(match[0]);
  const href = safeHref(text);
  return href ? { text, href } : null;
}

function trimUrl(url: string): string {
  let end = url.length;
  for (;;) {
    const c = url[end - 1];
    if (".,:;!?'\"*_~|".includes(c)) {
      end--;
      continue;
    }
    const open = c === ")" ? "(" : c === "]" ? "[" : null;
    if (open) {
      const head = url.slice(0, end);
      if (head.split(c).length > head.split(open).length) {
        end--;
        continue;
      }
    }
    return url.slice(0, end);
  }
}

/** The link target when it is http(s) or mailto with somewhere to go. */
export function safeHref(raw: string): string | null {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return null;
  }
  if (url.protocol === "http:" || url.protocol === "https:")
    return url.hostname ? url.href : null;
  if (url.protocol === "mailto:") return url.pathname ? url.href : null;
  return null;
}

/** Link targets that may get a preview: bare http(s) links outside code and
 *  spoilers, in order, without duplicates, at most `max`. */
export function previewUrls(blocks: Block[], max = 3): string[] {
  const found: string[] = [];
  const walk = (nodes: Inline[]) => {
    for (const node of nodes) {
      if (found.length >= max) return;
      if (node.type === "link") {
        if (
          !node.suppressed &&
          !node.href.startsWith("mailto:") &&
          !found.includes(node.href)
        )
          found.push(node.href);
      } else if ("children" in node && node.type !== "spoiler")
        walk(node.children);
    }
  };
  for (const block of blocks)
    if (block.type !== "codeblock") walk(block.children);
  return found;
}
