/** Up to two initials for the avatar fallback; `?` when the name is blank. */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const letters = parts
    .slice(0, 2)
    .map((part) => Array.from(part)[0] ?? "")
    .join("");
  return letters.toUpperCase() || "?";
}

/**
 * A server tile's letters. Like `initials`, but a one-word name keeps its
 * second letter ("Gaming" → "Ga", "Gilde" → "Gi"), so servers that start
 * alike stay apart in the rail.
 */
export function tileInitials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length !== 1) return initials(name);
  const [first = "", second = ""] = Array.from(parts[0] ?? "");
  return first.toUpperCase() + second.toLowerCase();
}

/**
 * Which of the eight `--lr-tile-*` tints a server gets: stable per id, so a
 * server keeps its colour on every screen.
 */
export function tileTone(id: string): number {
  let hash = 0;
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return (hash % 8) + 1;
}

/** The tint from `tileTone` as a CSS value (`var(--lr-tile-N)`). */
export function tileTint(id: string): string {
  return `var(--lr-tile-${tileTone(id)})`;
}

/**
 * The tile tint for a person's initials, picked by the name. A mixing hash
 * (FNV-1a with a final scramble): the server hash above leaves short names
 * bunched on a few tints. Servers keep theirs so their colours stay put.
 */
export function personTint(name: string): string {
  let hash = 0x811c9dc5;
  for (const char of name) {
    hash ^= char.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193);
  }
  hash ^= hash >>> 16;
  hash = Math.imul(hash, 0x45d9f3b);
  hash ^= hash >>> 16;
  return `var(--lr-tile-${((hash >>> 0) % 8) + 1})`;
}
