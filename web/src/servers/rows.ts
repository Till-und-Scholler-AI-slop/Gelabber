// The channel sidebar renders one flat list (so it can be virtualised):
// uncategorised channels first, then each category followed by its channels.

import { translate, translateIn } from "../i18n/catalog.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import type { Category, Channel, ServerDetail } from "./types.ts";

export type Row =
  | { kind: "category"; key: string; category: Category }
  | { kind: "channel"; key: string; channel: Channel }
  | { kind: "empty"; key: string; categoryId: string };

export const CATEGORY_PX = 40;
export const CHANNEL_PX = 34;
export const EMPTY_PX = 28;

export function rowHeight(row: Row | undefined): number {
  if (!row) return CHANNEL_PX;
  if (row.kind === "category") return CATEGORY_PX;
  if (row.kind === "empty") return EMPTY_PX;
  return CHANNEL_PX;
}

export function buildRows(server: ServerDetail): Row[] {
  const rows: Row[] = [];
  for (const channel of server.channels) {
    if (channel.category_id === null)
      rows.push({ kind: "channel", key: channel.id, channel });
  }
  for (const category of server.categories) {
    rows.push({ kind: "category", key: category.id, category });
    let count = 0;
    for (const channel of server.channels) {
      if (channel.category_id === category.id) {
        rows.push({ kind: "channel", key: channel.id, channel });
        count += 1;
      }
    }
    if (count === 0)
      rows.push({
        kind: "empty",
        key: `empty:${category.id}`,
        categoryId: category.id,
      });
  }
  return rows;
}

export type SectionRow = { kind: "section"; key: string; label: string };

/** The uncategorised voice rooms' heading, in the interface's language. */
export function voiceHeading(): string {
  return translate(servers, "voiceHeading");
}

/** The heading in every language: a server keeps the names its creator's
 * language gave it, and every viewer must merge them. */
const VOICE_HEADINGS = new Set(
  (["de", "en"] as const).map((locale) =>
    translateIn(locale, servers, "voiceHeading").toLowerCase(),
  ),
);

/**
 * The category whose name the voice heading would repeat (the first), in
 * either language: "Sprachkanäle" or "Voice channels".
 */
export function voiceHeadingCategory(
  categories: readonly Category[],
): Category | undefined {
  return categories.find((category) =>
    VOICE_HEADINGS.has(category.name.trim().toLowerCase()),
  );
}

/**
 * The sidebar's order: uncategorised text channels, the uncategorised voice
 * rooms under a "Sprachkanäle" heading of their own, then the categories.
 * A category of that name (in either language) takes the rooms at its top
 * instead, so the heading does not show twice. Only the list merges: the rooms stay
 * uncategorised, so a channel created from one of them is uncategorised too,
 * and deleting that category leaves them under their own heading again.
 */
export function sidebarRows(server: ServerDetail): (Row | SectionRow)[] {
  const base = buildRows(server);
  const loose = (kind: Channel["kind"]) =>
    base.filter(
      (row) =>
        row.kind === "channel" &&
        row.channel.category_id === null &&
        row.channel.kind === kind,
    );
  const voice = loose("voice");
  const categorised = base.filter(
    (row) => row.kind !== "channel" || row.channel.category_id !== null,
  );
  const home = voice.length
    ? voiceHeadingCategory(server.categories)
    : undefined;
  if (home)
    return [
      ...loose("text"),
      ...categorised.flatMap((row) =>
        row.kind === "category" && row.category.id === home.id
          ? [row, ...voice]
          : row.kind === "empty" && row.categoryId === home.id
            ? []
            : [row],
      ),
    ];
  return [
    ...loose("text"),
    ...(voice.length
      ? [
          {
            kind: "section" as const,
            key: "voice-heading",
            label: voiceHeading(),
          },
          ...voice,
        ]
      : []),
    ...categorised,
  ];
}

/** The channels in the order the sidebar lists them. */
export function channelOrder(server: ServerDetail): Channel[] {
  return sidebarRows(server).flatMap((row) =>
    row.kind === "channel" ? [row.channel] : [],
  );
}

/** First text channel, preferring the remembered one if it still exists. */
export function pickChannel(
  server: ServerDetail,
  remembered: string | undefined,
): Channel | undefined {
  if (remembered) {
    const hit = server.channels.find((c) => c.id === remembered);
    if (hit) return hit;
  }
  return server.channels.find((c) => c.kind === "text") ?? server.channels[0];
}
