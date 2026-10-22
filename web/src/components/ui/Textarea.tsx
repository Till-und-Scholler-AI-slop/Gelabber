import type { Ref, TextareaHTMLAttributes } from "react";

import "./ui.css";

export type TextareaProps = TextareaHTMLAttributes<HTMLTextAreaElement> & {
  /** Marks the field invalid: a danger border and `aria-invalid`. */
  invalid?: boolean;
  ref?: Ref<HTMLTextAreaElement>;
};

export function Textarea({ invalid, className, ...rest }: TextareaProps) {
  return (
    <textarea
      aria-invalid={invalid || undefined}
      className={["ui-field", className].filter(Boolean).join(" ")}
      {...rest}
    />
  );
}
