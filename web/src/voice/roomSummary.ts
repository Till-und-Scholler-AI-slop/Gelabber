import { translate } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";

/** "Mia", "Mia und Lena", "Mia, Lena und Kim", "Mia, Lena und 2 weitere".
 * `dative` after "mit": "Mia, Lena und 2 weiteren". */
export function nameList(
  names: readonly string[],
  { dative = false }: { dative?: boolean } = {},
): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2)
    return translate(voice, "names.two", { a: names[0], b: names[1] });
  if (names.length === 3)
    return translate(voice, "names.three", {
      a: names[0],
      b: names[1],
      c: names[2],
    });
  return translate(voice, dative ? "names.moreDative" : "names.more", {
    a: names[0],
    b: names[1],
    more: names.length - 2,
  });
}

/** The viewer among the names of `liveSentence`: "Du" / "You". */
export function youName(): string {
  return translate(voice, "names.you");
}

/** "Mia ist live", "Mia und Lena sind live", "Du bist live", "Du und Mia
 * seid live". With `self`, the first name is the viewer's (`youName`). */
export function liveSentence(names: readonly string[], self: boolean): string {
  const key = self
    ? names.length === 1
      ? "isLive.self"
      : "isLive.selfAndOthers"
    : names.length === 1
      ? "isLive.one"
      : "isLive.many";
  return translate(voice, key, { names: nameList(names) });
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
      ? translate(voice, "summary.onlyYou")
      : translate(voice, "summary.youWith", {
          names: nameList(others, { dative: true }),
        });
  if (others.length === 0) return translate(voice, "summary.nobody");
  const there = translate(
    voice,
    others.length === 1 ? "summary.thereOne" : "summary.thereMany",
    { names: nameList(others) },
  );
  if (live.length === 0) return there;
  return translate(voice, "summary.thereAndLive", {
    there,
    live: liveSentence(live, false),
  });
}

/** The header's facts while pictures run: how many are in the channel and
 * how many Lives run, the viewer's included, or else how many cameras are
 * on. "3 im Kanal · 1 Live", "3 im Kanal · 2 Kameras", "3 im Kanal". */
export function stageFacts({
  people,
  cameras,
  lives,
}: {
  people: number;
  cameras: number;
  lives: number;
}): string {
  const count = translate(voice, "facts.people", { count: people });
  if (lives > 0)
    return `${count} · ${translate(voice, "facts.live", { count: lives })}`;
  if (cameras === 0) return count;
  return `${count} · ${translate(voice, "facts.cameras", { count: cameras })}`;
}
