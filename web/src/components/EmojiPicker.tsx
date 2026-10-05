import { useId, useMemo, useState } from "react";
import data from "../messages/emoji-data.json";

const groups = [...new Set(data.emoji.map((entry) => entry.group))];
export default function EmojiPicker({
  onSelect,
}: {
  onSelect: (emoji: string) => void;
}) {
  const id = useId();
  const [query, setQuery] = useState("");
  const [group, setGroup] = useState("");
  const [limit, setLimit] = useState(80);
  const matches = useMemo(() => {
    const terms = query
      .trim()
      .toLocaleLowerCase("de")
      .split(/\s+/)
      .filter(Boolean);
    return data.emoji.filter(
      (entry) =>
        (!group || entry.group === group) &&
        terms.every((term) =>
          `${entry.emoji} ${entry.label} ${entry.keywords.join(" ")}`
            .toLocaleLowerCase("de")
            .includes(term),
        ),
    );
  }, [query, group]);
  return (
    <div className="flex flex-col gap-3">
      <label htmlFor={`${id}-search`} className="text-sm font-medium">
        Emoji suchen
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
        Kategorie
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
        <option value="">Alle Kategorien</option>
        {groups.map((label) => (
          <option key={label} value={label}>
            {label}
          </option>
        ))}
      </select>
      <p role="status" className="text-xs text-neutral-500">
        {matches.length} Emojis gefunden
      </p>
      <div
        className="grid max-h-[40dvh] grid-cols-5 gap-1 min-[380px]:grid-cols-6 overflow-y-auto sm:grid-cols-8"
        aria-label="Emoji-Auswahl"
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
          Weitere Emojis anzeigen
        </button>
      )}
      <a
        href="/emoji-license.txt"
        target="_blank"
        rel="noreferrer"
        className="text-xs text-neutral-500 underline"
      >
        Unicode {data.version} / CLDR {data.cldr} – Lizenz
      </a>
    </div>
  );
}
