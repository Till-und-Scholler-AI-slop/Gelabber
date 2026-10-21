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

/** One of the eight theme tile tints (`--lr-tile-1…8`), stable per id. */
export function tileTint(id: string): string {
  let hash = 0;
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) | 0;
  return `var(--lr-tile-${(Math.abs(hash) % 8) + 1})`;
}
