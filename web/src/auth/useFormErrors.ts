import { useCallback, useState } from "react";

import { ApiError } from "../api/client.ts";
import { errorMessage, fieldMessages } from "./rules.ts";

export type FormErrors = {
  form: string | null;
  fields: Record<string, string>;
};

const EMPTY: FormErrors = { form: null, fields: {} };

/**
 * Inline error state for a form: per-field messages plus one form-level
 * message. `fromError` maps an `ApiError` (or anything thrown) onto it.
 */
export function useFormErrors() {
  const [errors, setErrors] = useState<FormErrors>(EMPTY);

  const clear = useCallback(() => setErrors(EMPTY), []);

  const clearField = useCallback((field: string) => {
    setErrors((current) => {
      if (!(field in current.fields) && current.form === null) return current;
      const fields = { ...current.fields };
      delete fields[field];
      return { form: null, fields };
    });
  }, []);

  const setFields = useCallback((fields: Record<string, string>) => {
    setErrors({ form: null, fields });
  }, []);

  const fromError = useCallback((error: unknown) => {
    if (error instanceof ApiError) {
      const fields = fieldMessages(error.fields);
      const hasFieldErrors = Object.keys(fields).length > 0;
      setErrors({
        form: hasFieldErrors ? null : errorMessage(error.code),
        fields,
      });
      return;
    }
    if (error instanceof DOMException && error.name === "AbortError") {
      return;
    }
    setErrors({ form: errorMessage("internal"), fields: {} });
  }, []);

  return { errors, clear, clearField, setFields, fromError };
}
