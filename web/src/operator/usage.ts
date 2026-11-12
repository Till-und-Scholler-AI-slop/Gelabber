// The numbers of "Auslastung" as labelled groups: every field of
// `GET /api/operator/usage`, none added. A list so tests see exactly what
// the page shows.

import { translate } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import { formatBytes, formatCount } from "./format.ts";
import type { Usage } from "./types.ts";

export type Stat = {
  key: string;
  label: string;
  value: string;
};

export type Group = { key: string; title: string; stats: Stat[] };

type UsageKey = Extract<keyof typeof operator.de, `usage.${string}`>;

/** Every number of the response, in the order the page shows them. */
export function usageGroups(usage: Usage): Group[] {
  const t = (key: UsageKey) => translate(operator, key);
  const count = formatCount;
  return [
    {
      key: "accounts",
      title: t("usage.group.accounts"),
      stats: [
        {
          key: "total",
          label: t("usage.accounts.total"),
          value: count(usage.accounts.total),
        },
        {
          key: "new7d",
          label: t("usage.accounts.new7d"),
          value: count(usage.accounts.new_7d),
        },
        {
          key: "new30d",
          label: t("usage.accounts.new30d"),
          value: count(usage.accounts.new_30d),
        },
        {
          key: "withLiveSession",
          label: t("usage.accounts.withLiveSession"),
          value: count(usage.accounts.with_live_session),
        },
        {
          key: "pending",
          label: t("usage.accounts.pending"),
          value: count(usage.accounts.pending),
        },
        {
          key: "disabled",
          label: t("usage.accounts.disabled"),
          value: count(usage.accounts.disabled),
        },
      ],
    },
    {
      key: "servers",
      title: t("usage.group.servers"),
      stats: [
        {
          key: "total",
          label: t("usage.servers.total"),
          value: count(usage.servers.total),
        },
        {
          key: "blocked",
          label: t("usage.servers.blocked"),
          value: count(usage.servers.blocked),
        },
        {
          key: "delisted",
          label: t("usage.servers.delisted"),
          value: count(usage.servers.delisted),
        },
      ],
    },
    {
      key: "messages",
      title: t("usage.group.messages"),
      stats: [
        {
          key: "server24h",
          label: t("usage.messages.server24h"),
          value: count(usage.messages.server_24h),
        },
        {
          key: "server7d",
          label: t("usage.messages.server7d"),
          value: count(usage.messages.server_7d),
        },
        {
          key: "dm7d",
          label: t("usage.messages.dm7d"),
          value: count(usage.messages.dm_7d),
        },
      ],
    },
    {
      key: "storage",
      title: t("usage.group.storage"),
      stats: [
        {
          key: "attachmentBytes",
          label: t("usage.storage.attachmentBytes"),
          value: formatBytes(usage.storage.attachment_bytes),
        },
        {
          key: "attachmentCount",
          label: t("usage.storage.attachmentCount"),
          value: count(usage.storage.attachment_count),
        },
        {
          key: "uploadedToday",
          label: t("usage.storage.uploadedToday"),
          value: formatBytes(usage.storage.uploaded_today_bytes),
        },
      ],
    },
    {
      key: "now",
      title: t("usage.group.now"),
      stats: [
        {
          key: "sockets",
          label: t("usage.now.sockets"),
          value: count(usage.now.sockets_this_process),
        },
        {
          key: "inVoice",
          label: t("usage.now.inVoice"),
          value: count(usage.now.in_voice),
        },
        {
          key: "live",
          label: t("usage.now.live"),
          value: count(usage.now.live),
        },
      ],
    },
  ];
}
