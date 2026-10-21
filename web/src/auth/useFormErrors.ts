import { useCallback, useState, type RefObject } from "react";

import { ApiError } from "../api/client.ts";
import { errorMessage, fieldMessages } from "./rules.ts";
import { useCooldown } from "./useCooldown.ts";

export type FormErrors = {
  form: string | null;
  fields: Record<string, string>;
};

const EMPTY: FormErrors = { form: null, fields: {} };

/**
 * The one announced line for a set of field errors. A single error needs
 * none: focus moves to its field, which reads it out. Several get a summary,
 * so a screen reader hears one alert instead of one per field.
 */
export function fieldSummary(fields: Record<string, string>): string | null {
  return Object.keys(fields).length > 1
    ? "Bitte die markierten Felder prüfen."
    : null;
}

/** Moves focus to the first invalid control, in document order. */
export function focusFirstInvalid(root: ParentNode | null | undefined): void {
  root?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
}

/**
 * Inline error state for a form: per-field messages plus one form-level
 * message. `fromError` maps an `ApiError` (or anything thrown) onto it.
 * With `form`, focus moves to the first invalid field once errors render.
 * A rate limit with a known wait starts `cooldown`, the seconds until the
 * form may be sent again.
 */
export function useFormErrors(form?: RefObject<HTMLFormElement | null>) {
  const [errors, setErrors] = useState<FormErrors>(EMPTY);
  const { seconds: cooldown, start: startCooldown } = useCooldown();

  const focusInvalid = useCallback(() => {
    if (!form || typeof requestAnimationFrame === "undefined") return;
    requestAnimationFrame(() => focusFirstInvalid(form.current));
  }, [form]);

  const clear = useCallback(() => setErrors(EMPTY), []);

  const clearField = useCallback((field: string) => {
    setErrors((current) => {
      if (!(field in current.fields) && current.form === null) return current;
      const fields = { ...current.fields };
      delete fields[field];
      return { form: null, fields };
    });
  }, []);

  const setFields = useCallback(
    (fields: Record<string, string>) => {
      setErrors({ form: fieldSummary(fields), fields });
      focusInvalid();
    },
    [focusInvalid],
  );

  const fromError = useCallback(
    (error: unknown) => {
      if (error instanceof ApiError) {
        if (error.code === "rate_limited" && error.retryAfter) {
          const seconds = Math.ceil(error.retryAfter);
          startCooldown(seconds);
          setErrors({
            form: `Zu viele Versuche. Bitte ${seconds} Sekunden warten.`,
            fields: {},
          });
          return;
        }
        const fields = fieldMessages(error.fields);
        const hasFieldErrors = Object.keys(fields).length > 0;
        setErrors({
          form: hasFieldErrors
            ? fieldSummary(fields)
            : errorMessage(error.code),
          fields,
        });
        if (hasFieldErrors) focusInvalid();
        return;
      }
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      setErrors({ form: errorMessage("internal"), fields: {} });
    },
    [focusInvalid, startCooldown],
  );

  return { errors, clear, clearField, setFields, fromError, cooldown };
}
