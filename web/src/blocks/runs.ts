// Runs of messages by people the user blocks (#565), which a server channel
// folds into one "Blockierte Nachricht · Anzeigen" line each.

export interface BlockedRuns {
  /** For each row, the index of its run's first row (-1 outside a run). */
  start: number[];
  /** Each run's length, by its first row. */
  length: Map<number, number>;
}

export function blockedRuns(
  items: readonly { author: { id: string } }[],
  blocked: ReadonlySet<string>,
): BlockedRuns | null {
  if (blocked.size === 0) return null;
  const start: number[] = [];
  const length = new Map<number, number>();
  items.forEach((message, index) => {
    if (!blocked.has(message.author.id)) {
      start.push(-1);
      return;
    }
    const before = index > 0 ? (start[index - 1] ?? -1) : -1;
    const first = before === -1 ? index : before;
    start.push(first);
    length.set(first, (length.get(first) ?? 0) + 1);
  });
  return { start, length };
}
