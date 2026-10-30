/** "Mia", "Mia und Lena", "Mia, Lena und Kim", "Mia, Lena und 2 weitere". */
export function nameList(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} und ${names[1]}`;
  if (names.length === 3) return `${names[0]}, ${names[1]} und ${names[2]}`;
  return `${names[0]}, ${names[1]} und ${names.length - 2} weitere`;
}

/** "Mia ist live", "Mia und Lena sind live", "Du bist live", "Du und Mia
 * seid live". */
export function liveSentence(names: readonly string[], self: boolean): string {
  const verb = self
    ? names.length === 1
      ? "bist"
      : "seid"
    : names.length === 1
      ? "ist"
      : "sind";
  return `${nameList(names)} ${verb} live`;
}

/** The room's one line about who is there: occupancy, never a claim that
 * anyone is talking. `others` leaves the viewer out, and so does `live`,
 * who of them is live: outside, the room says so, since a Live is
 * watched inside. The way in is the button under it, not this line. */
export function roomSummary(
  others: readonly string[],
  { here, live = [] }: { here: boolean; live?: readonly string[] },
): string {
  if (here)
    return others.length === 0
      ? "Nur du bist im Kanal."
      : `Du bist hier mit ${nameList(others)}.`;
  if (others.length === 0) return "Niemand im Kanal.";
  const there = `${nameList(others)} ${others.length === 1 ? "ist" : "sind"} da.`;
  if (live.length === 0) return there;
  return `${there} ${nameList(live)} ${live.length === 1 ? "ist" : "sind"} live.`;
}

/** The stage's line while pictures run: how many are in the channel and
 * who is live, or how many cameras are on. "3 im Kanal · 2 Kameras",
 * "3 im Kanal · Jonas ist live". `live` names the others; `selfLive` adds
 * the viewer as "Du". */
export function stageFacts({
  people,
  cameras,
  live,
  selfLive,
}: {
  people: number;
  cameras: number;
  live: readonly string[];
  selfLive: boolean;
}): string {
  const count = `${people} im Kanal`;
  if (live.length > 0 || selfLive)
    return `${count} · ${liveSentence(selfLive ? ["Du", ...live] : live, selfLive)}`;
  if (cameras === 0) return count;
  return `${count} · ${cameras} ${cameras === 1 ? "Kamera" : "Kameras"}`;
}
