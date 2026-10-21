import type { InputHTMLAttributes, Ref } from "react";

import "./ui.css";

export type InputProps = InputHTMLAttributes<HTMLInputElement> & {
  /** Marks the field invalid: a danger border and `aria-invalid`. */
  invalid?: boolean;
  ref?: Ref<HTMLInputElement>;
};

export function Input({ invalid, className, ...rest }: InputProps) {
  return (
    <input
      aria-invalid={invalid || undefined}
      className={["ui-field", className].filter(Boolean).join(" ")}
      {...rest}
    />
  );
}
