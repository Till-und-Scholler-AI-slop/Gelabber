// Which rows of a conversation just arrived live, so they can fade in.
// Only a few new rows below what was already shown count: not the first
// load, not older pages above, not a jump that swaps the list, and not a
// catch-up after a reconnect.

/** `live`: someone's new message, or one's own on its way. `confirmed`:
 * one's own, now saved, taking over from its `tmp:` row. */
export type Arrival = "live" | "confirmed";

/** More new rows than this at once are a catch-up, not a conversation. */
export const LIVE_ARRIVALS_MAX = 3;

type Row = { id: string; author: { id: string } };

const isTmp = (row: Row) => row.id.startsWith("tmp:");

export function arrivals(
  previous: readonly Row[],
  next: readonly Row[],
  me: string | undefined,
): ReadonlyMap<string, Arrival> {
  const found = new Map<string, Arrival>();
  // The newest row that was already there and saved: new ones come after.
  const newest = previous.findLast((row) => !isTmp(row));
  if (!newest) return found;
  const at = next.findIndex((row) => row.id === newest.id);
  if (at < 0) return found;
  const before = new Set(previous.map((row) => row.id));
  const added = next.slice(at + 1).filter((row) => !before.has(row.id));
  if (added.length === 0 || added.length > LIVE_ARRIVALS_MAX) return found;
  // One's own saved message while a send was on its way is that send: the
  // server's copy, which takes over from its tmp row now or a moment later.
  let confirmed = previous.filter(isTmp).length;
  for (const row of added) {
    const own = me !== undefined && row.author.id === me && !isTmp(row);
    if (own && confirmed > 0) {
      found.set(row.id, "confirmed");
      confirmed -= 1;
    } else found.set(row.id, "live");
  }
  return found;
}
