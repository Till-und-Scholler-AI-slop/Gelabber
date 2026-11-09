// Which words of a hit the search matched. The server searches with
// `websearch_to_tsquery('simple', q)` (messages/workflows.rs): whole words,
// any case, `"…"` for a phrase, `-word` to leave out and `or` between
// alternatives. A word here is a run of letters and digits, as the parser
// splits text.

const WORD = /[\p{L}\p{N}]+/gu;

/** The words of `q` a hit can contain, lower-cased. */
export function searchTerms(q: string): string[] {
  const terms = new Set<string>();
  for (const token of q.replace(/"/g, " ").split(/\s+/)) {
    if (!token || token.startsWith("-") || token.toLowerCase() === "or")
      continue;
    for (const [word] of token.matchAll(WORD)) terms.add(word.toLowerCase());
  }
  return [...terms];
}

export type MarkedRun = { text: string; match: boolean };

/** `text` in runs, `match` where one of `terms` stands as a whole word. */
export function markMatches(
  text: string,
  terms: readonly string[],
): MarkedRun[] {
  if (!terms.length) return [{ text, match: false }];
  const wanted = new Set(terms);
  const runs: MarkedRun[] = [];
  let at = 0;
  for (const found of text.matchAll(WORD)) {
    if (!wanted.has(found[0].toLowerCase())) continue;
    const start = found.index;
    if (start > at) runs.push({ text: text.slice(at, start), match: false });
    runs.push({ text: found[0], match: true });
    at = start + found[0].length;
  }
  if (at < text.length) runs.push({ text: text.slice(at), match: false });
  return runs;
}
