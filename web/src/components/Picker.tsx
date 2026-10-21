import { useEffect, useId, useState, type ReactNode } from "react";

import { Input } from "./ui/Input.tsx";
import "./shell.css";

export type PickerOption<T> = {
  id: string;
  value: T;
  /** Read out and shown as the option's name. */
  label: string;
  /** A second, quieter line: where it is. */
  detail?: string | null;
  icon: ReactNode;
  /** Badges after the name. */
  trailing?: ReactNode;
};

/**
 * A search field over a list to pick from (combobox and listbox): ↑ and ↓
 * move the highlight, Enter picks it, a click or tap picks any option. The
 * caller filters `options` by `query`.
 */
export function Picker<T>({
  label,
  placeholder,
  query,
  onQuery,
  options,
  onPick,
  empty,
  busy = false,
}: {
  label: string;
  placeholder: string;
  query: string;
  onQuery: (query: string) => void;
  options: readonly PickerOption<T>[];
  onPick: (option: PickerOption<T>) => void;
  /** Shown instead of the list when nothing matches. */
  empty: ReactNode;
  /** A pick is on its way: picking again waits. */
  busy?: boolean;
}) {
  const id = useId();
  const listId = `${id}-list`;
  const [active, setActive] = useState(0);
  const current = Math.min(active, options.length - 1);
  const activeOption = current >= 0 ? options[current] : undefined;
  const optionId = (option: PickerOption<T>) => `${id}-${option.id}`;

  useEffect(() => {
    if (!activeOption) return;
    document
      .getElementById(`${id}-${activeOption.id}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [id, activeOption]);

  const pick = (option: PickerOption<T> | undefined) => {
    if (option && !busy) onPick(option);
  };

  return (
    <div className="shell-picker">
      <label htmlFor={`${id}-field`} className="sr-only">
        {label}
      </label>
      <Input
        id={`${id}-field`}
        type="search"
        role="combobox"
        aria-expanded={options.length > 0}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={
          activeOption ? optionId(activeOption) : undefined
        }
        aria-busy={busy || undefined}
        autoComplete="off"
        spellCheck={false}
        enterKeyHint="go"
        placeholder={placeholder}
        value={query}
        onChange={(event) => {
          onQuery(event.target.value);
          setActive(0);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const step = event.key === "ArrowDown" ? 1 : -1;
            setActive(
              Math.max(0, Math.min(options.length - 1, current + step)),
            );
          } else if (event.key === "Enter") {
            event.preventDefault();
            pick(activeOption);
          }
        }}
      />
      {options.length > 0 ? (
        <ul
          id={listId}
          role="listbox"
          aria-label={label}
          className="shell-picker-list"
        >
          {options.map((option, index) => (
            <li
              key={option.id}
              id={optionId(option)}
              role="option"
              aria-selected={index === current}
              className={`shell-picker-option${index === current ? " is-active" : ""}`}
              onPointerMove={() => setActive(index)}
              // Keeps focus (and the keyboard on a phone) in the field.
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => pick(option)}
            >
              <span className="shell-picker-icon">{option.icon}</span>
              <span className="shell-picker-text">
                <span className="shell-picker-label">{option.label}</span>
                {option.detail ? (
                  <span className="shell-picker-detail">{option.detail}</span>
                ) : null}
              </span>
              {option.trailing}
            </li>
          ))}
        </ul>
      ) : null}
      {/* Read out as the list changes: how many match, or why none does. */}
      <p
        className={options.length > 0 ? "sr-only" : "shell-picker-empty"}
        role="status"
      >
        {options.length > 0 ? `${options.length} Treffer` : empty}
      </p>
    </div>
  );
}
