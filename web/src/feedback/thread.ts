// The settings area's pure parts: what went along as one line, and which
// entries and messages a visit marks read or keeps as "Neu".

import { OS_NAMES } from "../auth/devices.ts";
import { translate } from "../i18n/catalog.ts";
import { feedback } from "../i18n/catalogs/feedback.ts";
import { language } from "../i18n/catalogs/language.ts";
import type { EntryContext, FeedbackEntry } from "./types.ts";

/** "Web-App · Linux · 0.10.0 · Deutsch · /s/…": what went along, in a line. */
export function sentAlongLine(context: EntryContext): string {
  const parts = [
    translate(
      feedback,
      context.client === "desktop" ? "context.desktopApp" : "context.webApp",
    ),
    context.os === null
      ? null
      : context.os === "other"
        ? translate(feedback, "context.unknownSystem")
        : OS_NAMES[context.os],
    context.app_version,
    translate(language, `option.${context.locale}`),
    context.page,
  ];
  return parts.filter(Boolean).join(" · ");
}

/** `marks` with the new messages of `entries` added; the same set when
 * there are none it lacks. */
export function rememberNew(
  marks: ReadonlySet<string>,
  entries: readonly FeedbackEntry[],
): ReadonlySet<string> {
  const fresh = entries
    .flatMap((entry) => entry.messages)
    .filter((message) => message.new && !marks.has(message.id))
    .map((message) => message.id);
  return fresh.length === 0 ? marks : new Set([...marks, ...fresh]);
}

/** The entries with unread replies not yet marked read in this visit,
 * each with its newest message on screen (`through` of the read call). */
export function entriesToRead(
  entries: readonly FeedbackEntry[],
  asked: ReadonlySet<string>,
): { id: string; through: string }[] {
  return entries.flatMap((entry) => {
    const last = entry.messages.at(-1);
    return entry.unread && last && !asked.has(entry.id)
      ? [{ id: entry.id, through: last.id }]
      : [];
  });
}
