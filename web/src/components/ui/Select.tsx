import type { Ref, SelectHTMLAttributes } from "react";

import { ChevronIcon } from "../Icons.tsx";
import "./ui.css";

export type SelectProps = SelectHTMLAttributes<HTMLSelectElement> & {
  /** Marks the field invalid: a danger border and `aria-invalid`. */
  invalid?: boolean;
  ref?: Ref<HTMLSelectElement>;
};

/** A native select drawn like the other fields, with its own chevron. */
export function Select({ invalid, className, ...rest }: SelectProps) {
  return (
    <span className="ui-select">
      <select
        aria-invalid={invalid || undefined}
        className={["ui-field", className].filter(Boolean).join(" ")}
        {...rest}
      />
      <ChevronIcon size={14} />
    </span>
  );
}
