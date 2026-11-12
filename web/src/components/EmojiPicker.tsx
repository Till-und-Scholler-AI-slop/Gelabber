import { useId, useMemo, useState } from "react";
import { useT } from "../i18n/catalog.ts";
import { emoji as catalog } from "../i18n/catalogs/emoji.ts";
import { useCurrentLocale } from "../i18n/locale.ts";
import data from "../messages/emoji-data.json";
import { useEmojiNames } from "./emojiNames.ts";
import { useMissingEmoji } from "./emojiSupport.ts";

const groups = [...new Set(data.emoji.map((entry) => entry.group))];
const allEmoji = data.emoji.map((entry) => entry.emoji);
export default function EmojiPicker({
  onSelect,
}: {
  onSelect: (emoji: string) => void;
}) {
  const t = useT(catalog);
  // English names once they are loaded; German until then.
  const names = useEmojiNames(useCurrentLocale());
  const entries = useMemo(
    () =>
      names
        ? data.emoji.map((entry) => {
            const [label, keywords] = names.emoji[entry.emoji] ?? [
              entry.label,
              entry.keywords,
            ];
            return { ...entry, label, keywords };
          })
        : data.emoji,
    [names],
  );
  const id = useId();
  const [query, setQuery] = useState("");
  const [group, setGroup] = useState("");
  const [limit, setLimit] = useState(80);
  // Emoji newer than this device's emoji font would show as empty boxes.
  const missing = useMissingEmoji(allEmoji, data.version);
  const matches = useMemo(() => {
    const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
    return entries.filter(
      (entry) =>
        !missing.has(entry.emoji) &&
        (!group || entry.group === group) &&
        terms.every((term) =>
          `${entry.emoji} ${entry.label} ${entry.keywords.join(" ")}`
            .toLocaleLowerCase()
            .includes(term),
        ),
    );
  }, [entries, query, group, missing]);
  return (
    <div className="flex flex-col gap-3">
      <label htmlFor={`${id}-search`} className="text-sm font-medium">
        {t("picker.search")}
      </label>
      <input
        id={`${id}-search`}
        type="search"
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          setLimit(80);
        }}
        autoFocus
        className="rounded border border-neutral-300 p-2 dark:border-neutral-600"
      />
      <label htmlFor={`${id}-group`} className="text-sm font-medium">
        {t("picker.group")}
      </label>
      <select
        id={`${id}-group`}
        value={group}
        onChange={(event) => {
          setGroup(event.target.value);
          setLimit(80);
        }}
        className="rounded border border-neutral-300 p-2 dark:border-neutral-600"
      >
        <option value="">{t("picker.allGroups")}</option>
        {groups.map((value) => (
          <option key={value} value={value}>
            {names?.groups[value] ?? value}
          </option>
        ))}
      </select>
      <p role="status" className="text-xs text-neutral-500">
        {t("picker.found", { count: matches.length })}
      </p>
      <div
        className="grid max-h-[40dvh] grid-cols-5 gap-1 min-[380px]:grid-cols-6 overflow-y-auto sm:grid-cols-8"
        aria-label={t("picker.gridLabel")}
      >
        {matches.slice(0, limit).map((entry) => (
          <button
            key={entry.emoji}
            type="button"
            title={entry.label}
            aria-label={entry.label}
            onClick={() => onSelect(entry.emoji)}
            className="min-h-11 min-w-11 rounded text-2xl hover:bg-neutral-100 focus-visible:outline-2 dark:hover:bg-neutral-800"
          >
            <span aria-hidden="true">{entry.emoji}</span>
          </button>
        ))}
      </div>
      {matches.length > limit && (
        <button
          type="button"
          onClick={() => setLimit(limit + 80)}
          className="rounded border px-3 py-2 text-sm"
        >
          {t("picker.more")}
        </button>
      )}
      <a
        href="/emoji-license.txt"
        target="_blank"
        rel="noreferrer"
        className="text-xs text-neutral-500 underline"
      >
        {t("picker.licence", { version: data.version, cldr: data.cldr })}
      </a>
    </div>
  );
}
