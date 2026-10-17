// The `@…` suggestions of the composer and the edit field (issue #241).
import { useId, useState, type KeyboardEvent } from "react";

import {
  insertMention,
  mentionAt,
  mentionSuggestions,
  type Suggestion,
} from "../messages/mentions.ts";
import { useMentions } from "./mentionContext.ts";

/** Suggestions for the `@…` at the caret of a field. */
export function useMentionSuggestions(
  draft: string,
  setDraft: (text: string, caret: number) => void,
) {
  const { members, me, keywords } = useMentions();
  const [caret, setCaret] = useState<number | null>(null);
  const [active, setActive] = useState(0);
  // Escape closes the list until the `@…` it was shown for changes.
  const [dismissed, setDismissed] = useState<string | null>(null);
  const listId = useId();
  const at = caret === null ? null : mentionAt(draft, caret);
  const key = at ? `${at.start}:${at.query}` : null;
  const suggestions =
    at && key !== dismissed
      ? mentionSuggestions(at.query, members, { me, keywords })
      : [];
  const open = suggestions.length > 0;
  const current = Math.min(active, suggestions.length - 1);

  const pick = (suggestion: Suggestion) => {
    if (!at) return;
    const next = insertMention(draft, at, suggestion);
    setDraft(next.text, next.caret);
    setCaret(next.caret);
    setActive(0);
  };

  /** True when the key was the list's. */
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!open || event.nativeEvent.isComposing) return false;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((current + step + suggestions.length) % suggestions.length);
      return true;
    }
    if ((event.key === "Enter" && !event.shiftKey) || event.key === "Tab") {
      event.preventDefault();
      pick(suggestions[current]);
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setDismissed(key);
      return true;
    }
    return false;
  };

  const track = (input: HTMLTextAreaElement) => {
    setCaret(
      input.selectionStart === input.selectionEnd ? input.selectionStart : null,
    );
    setActive(0);
  };

  const optionId = (index: number) => `${listId}-${index}`;
  const fieldProps = {
    role: "combobox",
    "aria-autocomplete": "list" as const,
    "aria-expanded": open,
    "aria-controls": open ? listId : undefined,
    "aria-activedescendant": open ? optionId(current) : undefined,
    onSelect: (event: { currentTarget: HTMLTextAreaElement }) =>
      track(event.currentTarget),
    onBlur: () => setCaret(null),
  };

  const list = open ? (
    <ul
      id={listId}
      role="listbox"
      className="lr-mention-list"
      aria-label="Erwähnen"
    >
      {suggestions.map((suggestion, index) => (
        <li
          key={suggestion.kind === "user" ? suggestion.id : suggestion.kind}
          id={optionId(index)}
          role="option"
          aria-selected={index === current}
          className={index === current ? "is-active" : undefined}
          // Before the field's blur, which would close the list.
          onPointerDown={(event) => {
            event.preventDefault();
            pick(suggestion);
          }}
          onPointerEnter={() => setActive(index)}
        >
          {suggestion.kind === "user" ? (
            <span>@{suggestion.name}</span>
          ) : (
            <>
              <span>@{suggestion.kind}</span>
              <small>
                {suggestion.kind === "everyone"
                  ? "Alle Mitglieder"
                  : "Alle, die gerade online sind"}
              </small>
            </>
          )}
        </li>
      ))}
    </ul>
  ) : null;

  return { fieldProps, onKeyDown, list, track };
}
