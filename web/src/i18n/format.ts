// Locale-bound formatting, in the interface's language (de-DE or en-GB).
// Every date, time, number and name sort that people read goes through
// here instead of naming a locale itself. Search folding
// (`toLocaleLowerCase()`) stays locale-neutral and does not.

import { currentLocale, intlLocale } from "./locale.ts";

type DateInput = Date | number | string;

const cache = new Map<string, unknown>();

/** One formatter per language and options; building them is not cheap. */
function cached<T>(kind: string, options: object | undefined, make: () => T) {
  const key = `${kind}|${currentLocale()}|${JSON.stringify(options ?? {})}`;
  let formatter = cache.get(key) as T | undefined;
  if (!formatter) {
    formatter = make();
    cache.set(key, formatter);
  }
  return formatter;
}

const toDate = (value: DateInput) =>
  value instanceof Date ? value : new Date(value);

/** A date (and, with options asking for it, a time). */
export function formatDate(
  value: DateInput,
  options: Intl.DateTimeFormatOptions = {
    day: "numeric",
    month: "long",
    year: "numeric",
  },
): string {
  return cached(
    "date",
    options,
    () => new Intl.DateTimeFormat(intlLocale(currentLocale()), options),
  ).format(toDate(value));
}

/** A time of day, by default hours and minutes. */
export function formatTime(
  value: DateInput,
  options: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" },
): string {
  return cached(
    "time",
    options,
    () => new Intl.DateTimeFormat(intlLocale(currentLocale()), options),
  ).format(toDate(value));
}

/** "in 3 Tagen" / "in 3 days", "gestern" / "yesterday" with `numeric: "auto"`. */
export function formatRelative(
  value: number,
  unit: Intl.RelativeTimeFormatUnit,
  options: Intl.RelativeTimeFormatOptions = { numeric: "always" },
): string {
  return cached(
    "relative",
    options,
    () => new Intl.RelativeTimeFormat(intlLocale(currentLocale()), options),
  ).format(value, unit);
}

export function formatNumber(
  value: number,
  options?: Intl.NumberFormatOptions,
): string {
  return cached(
    "number",
    options,
    () => new Intl.NumberFormat(intlLocale(currentLocale()), options),
  ).format(value);
}

/** Sort order for names people read, as the interface's language sorts. */
export function compareNames(a: string, b: string): number {
  return cached(
    "collator",
    undefined,
    () => new Intl.Collator(intlLocale(currentLocale())),
  ).compare(a, b);
}
