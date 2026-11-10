// Numbers, sizes and dates of the operator pages, in the interface's
// language (through i18n/format.ts).

import { formatNumber, formatRelative } from "../i18n/format.ts";

const BYTE_UNITS = [
  "byte",
  "kilobyte",
  "megabyte",
  "gigabyte",
  "terabyte",
] as const;

/**
 * A size in decimal units: "6,8 GB" / "6.8 GB", "340 MB". One decimal below
 * ten, none above, so the column stays short.
 */
export function formatBytes(bytes: number): string {
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1000 && unit < BYTE_UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  // 999,6 MB would show as "1.000 MB": that is the next unit.
  if (Math.round(value) >= 1000 && unit < BYTE_UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return formatNumber(value, {
    style: "unit",
    unit: BYTE_UNITS[unit],
    maximumFractionDigits: unit > 0 && value < 10 ? 1 : 0,
  });
}

/** A count: "1.234" / "1,234". */
export function formatCount(count: number): string {
  return formatNumber(count);
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/**
 * How long ago (or until when) as words: "vor 5 Minuten", "gestern",
 * "in 3 Tagen". Under a minute is "in dieser Minute" / "this minute".
 */
export function formatWhen(
  value: string | number | Date,
  now: number = Date.now(),
): string {
  const at =
    value instanceof Date ? value.getTime() : new Date(value).getTime();
  const diff = at - now;
  const size = Math.abs(diff);
  const auto = { numeric: "auto" } as const;
  if (size < HOUR)
    return formatRelative(Math.trunc(diff / MINUTE), "minute", auto);
  if (size < DAY) return formatRelative(Math.trunc(diff / HOUR), "hour", auto);
  if (size < 30 * DAY)
    return formatRelative(Math.trunc(diff / DAY), "day", auto);
  if (size < 365 * DAY)
    return formatRelative(Math.trunc(diff / (30 * DAY)), "month", auto);
  return formatRelative(Math.trunc(diff / (365 * DAY)), "year", auto);
}
