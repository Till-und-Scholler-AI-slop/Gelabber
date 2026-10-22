import type { ReactNode } from "react";

import "./ui.css";

/**
 * Label, control and one line under it: the error when there is one,
 * otherwise the hint. The control takes `id` and `fieldDescription(id, …)`.
 * The error is not a live region: the form's `FormError` is the one summary
 * that is announced.
 */
export function FormField({
  id,
  label,
  hint,
  error,
  children,
  className,
}: {
  id: string;
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null | undefined;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={["ui-form-field", className].filter(Boolean).join(" ")}>
      <label htmlFor={id} className="ui-label">
        {label}
      </label>
      {children}
      {error ? (
        <p id={`${id}-error`} className="ui-field-error">
          {error}
        </p>
      ) : hint ? (
        <p id={`${id}-hint`} className="ui-hint">
          {hint}
        </p>
      ) : null}
    </div>
  );
}
