import type { InputHTMLAttributes, Ref } from "react";

import { dialogInitialFocus } from "../dialogFocus.ts";
import "./ui.css";

export type InputProps = InputHTMLAttributes<HTMLInputElement> & {
  /** Marks the field invalid: a danger border and `aria-invalid`. */
  invalid?: boolean;
  ref?: Ref<HTMLInputElement>;
};

/** `autoFocus` also picks the field a modal dialog opens on. */
export function Input({ invalid, className, ref, ...rest }: InputProps) {
  const focus = dialogInitialFocus(rest.autoFocus);
  return (
    <input
      aria-invalid={invalid || undefined}
      className={["ui-field", className].filter(Boolean).join(" ")}
      {...rest}
      ref={
        focus.ref && ref
          ? (element: HTMLInputElement | null) => {
              focus.ref(element);
              if (typeof ref === "function") return ref(element);
              ref.current = element;
            }
          : (focus.ref ?? ref)
      }
    />
  );
}
