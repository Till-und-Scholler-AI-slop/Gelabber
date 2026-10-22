/** "Mia", "Mia und Lena", "Mia, Lena und Kim", "Mia, Lena und 2 weitere". */
export function nameList(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} und ${names[1]}`;
  if (names.length === 3) return `${names[0]}, ${names[1]} und ${names[2]}`;
  return `${names[0]}, ${names[1]} und ${names.length - 2} weitere`;
}

/** The room's one line about who is there: occupancy, never a claim that
 * anyone is talking. `others` leaves the viewer out. */
export function roomSummary(
  others: readonly string[],
  { here, canJoin }: { here: boolean; canJoin: boolean },
): string {
  if (here)
    return others.length === 0
      ? "Du bist da. Sobald jemand dazukommt, hört ihr euch."
      : `Du bist hier mit ${nameList(others)}.`;
  if (others.length === 0)
    return canJoin
      ? "Noch niemand da – komm als Erste:r dazu."
      : "Gerade ist niemand da.";
  return `${nameList(others)} ${others.length === 1 ? "ist" : "sind"} da.`;
}
