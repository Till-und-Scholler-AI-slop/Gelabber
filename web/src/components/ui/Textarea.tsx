import type { Ref, TextareaHTMLAttributes } from "react";

import "./ui.css";

export type TextareaProps = TextareaHTMLAttributes<HTMLTextAreaElement> & {
  /** Marks the field invalid: a danger border and `aria-invalid`. */
  invalid?: boolean;
  /** Inside a {@link FieldShell}: no frame of its own, the shell draws it. */
  bare?: boolean;
  ref?: Ref<HTMLTextAreaElement>;
};

export function Textarea({ invalid, bare, className, ...rest }: TextareaProps) {
  return (
    <textarea
      aria-invalid={invalid || undefined}
      className={[bare ? "ui-field-bare" : "ui-field", className]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    />
  );
}
