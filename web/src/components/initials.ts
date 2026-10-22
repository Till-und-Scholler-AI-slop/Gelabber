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
 * Which of the eight `--lr-tile-*` tints a server gets: stable per id, so a
 * server keeps its colour on every screen.
 */
export function tileTone(id: string): number {
  let hash = 0;
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return (hash % 8) + 1;
}
