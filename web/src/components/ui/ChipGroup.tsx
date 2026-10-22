import { useRef, type ReactNode } from "react";

import { nextChipIndex } from "./chipKeys.ts";
import "./ui.css";

export type ChipOption<T extends string> = {
  value: T;
  label: ReactNode;
  disabled?: boolean;
};

/**
 * One-of-many choice as a row of chips: a radio group with a single Tab
 * stop, where the arrow keys move and select.
 */
export function ChipGroup<T extends string>({
  label,
  options,
  value,
  onChange,
  className,
}: {
  /** Names the group for assistive technology. */
  label: string;
  options: readonly ChipOption<T>[];
  value: T | null;
  onChange: (value: T) => void;
  className?: string;
}) {
  const chips = useRef<(HTMLButtonElement | null)[]>([]);
  const selected = options.findIndex((option) => option.value === value);
  const tabStop =
    selected >= 0 ? selected : options.findIndex((option) => !option.disabled);
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={["ui-chip-group", className].filter(Boolean).join(" ")}
    >
      {options.map((option, index) => (
        <button
          key={option.value}
          ref={(element) => {
            chips.current[index] = element;
          }}
          type="button"
          role="radio"
          aria-checked={index === selected}
          tabIndex={index === tabStop ? 0 : -1}
          disabled={option.disabled}
          className="ui-chip"
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => {
            const next = nextChipIndex(
              event.key,
              index,
              options.map((o) => Boolean(o.disabled)),
            );
            if (next === null) return;
            event.preventDefault();
            onChange(options[next].value);
            chips.current[next]?.focus();
          }}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
