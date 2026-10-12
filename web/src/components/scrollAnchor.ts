/** Where the list has to scroll so the row `anchor` names sits `offset` px
 * from the top again after rows were added above it or removed: older
 * history loading, a delete, retention trimming the oldest page. `undefined`
 * when that row is gone or not laid out. */
export function anchoredScrollTop(
  rows: readonly { id: string }[],
  anchor: { id: string; offset: number } | null,
  startOf: (index: number) => number | undefined,
): number | undefined {
  if (!anchor) return undefined;
  const index = rows.findIndex((row) => row.id === anchor.id);
  const start = index >= 0 ? startOf(index) : undefined;
  return start === undefined ? undefined : start - anchor.offset;
}
