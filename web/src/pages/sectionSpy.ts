// Which section of a one-page settings screen the reader is in, for the
// anchor row's `aria-current` (ServerSettingsPage; native
// server_settings::current_section).

/** How far below the anchor row a heading may sit and still count as
 * reached: a jump leaves it 15px down (`scroll-margin-top`). */
export const SECTION_LINE = 32;

export type SectionTop<T extends string> = {
  id: T;
  /** The heading's top, measured from the bottom of the anchor row. */
  top: number;
};

/**
 * The last section whose heading has reached the line under the anchor row.
 * Scrolled to the very end, the last section counts too: a short one at the
 * bottom never reaches the line.
 */
export function currentSection<T extends string>(
  headings: readonly SectionTop<T>[],
  atEnd: boolean,
): T | undefined {
  if (atEnd) return headings.at(-1)?.id;
  let current = headings[0]?.id;
  for (const heading of headings) {
    if (heading.top > SECTION_LINE) break;
    current = heading.id;
  }
  return current;
}

/** At the end of a scroller that scrolls at all. */
export function scrolledToEnd(
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
): boolean {
  return scrollTop > 0 && scrollTop + clientHeight >= scrollHeight - 2;
}
