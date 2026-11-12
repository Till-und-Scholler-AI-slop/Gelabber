// Day breaks and timestamps for the message list, in the interface's
// language (src/i18n/format.ts).

import { translate } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { formatDate, formatTime } from "../i18n/format.ts";
import type { Message } from "../messages/types.ts";

function dayKey(iso: string): string {
  const date = new Date(iso);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

export function dayBreak(
  previous: Message | undefined,
  message: Message,
): boolean {
  return (
    !previous || dayKey(previous.created_at) !== dayKey(message.created_at)
  );
}

export function dayLabel(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (dayKey(iso) === dayKey(today.toISOString()))
    return translate(chat, "day.today");
  if (dayKey(iso) === dayKey(yesterday.toISOString()))
    return translate(chat, "day.yesterday");
  return formatDate(date, {
    weekday: "long",
    day: "numeric",
    month: "long",
    ...(date.getFullYear() === today.getFullYear() ? {} : { year: "numeric" }),
  });
}

/** "14:02" today, "21.10., 14:02" on another day of this year and
 * "21.10.2025, 14:02" before it (in English "21/10, 14:02"): the message
 * list and search hits. */
export function formatWhen(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const thisYear = date.getFullYear() === now.getFullYear();
  const sameDay =
    thisYear &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return formatTime(date, {
    hour: "2-digit",
    minute: "2-digit",
    ...(sameDay
      ? {}
      : {
          day: "2-digit",
          month: "2-digit",
          ...(thisYear ? {} : { year: "numeric" }),
        }),
  });
}

/** "Dienstag, 21. Oktober 2026 um 14:02", for a time's tooltip. */
export function formatFull(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return formatDate(date, { dateStyle: "full", timeStyle: "short" });
}

/** "14:02", beside a grouped message. */
export function formatClock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return formatTime(date);
}
