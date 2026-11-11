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
    // English's short "byte" has no plural ("0 byte"); the long form does.
    unitDisplay: unit === 0 ? "long" : "short",
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
  const auto = { numeric: "auto" } as const;
  // Rounded, so an invitation made for 7 days reads "in 7 Tagen" at once;
  // the unit is the first one whose rounded count stays below the next.
  const steps = [
    [MINUTE, 60, "minute"],
    [HOUR, 24, "hour"],
    // Up to 30 days in days: the longest invitation reads "in 30 Tagen".
    [DAY, 31, "day"],
    [30 * DAY, 12, "month"],
  ] as const;
  for (const [size, below, unit] of steps) {
    const count = Math.round(diff / size);
    if (Math.abs(count) < below) return formatRelative(count, unit, auto);
  }
  return formatRelative(Math.round(diff / (365 * DAY)), "year", auto);
}
