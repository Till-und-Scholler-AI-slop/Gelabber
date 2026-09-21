import type { InputHTMLAttributes } from "react";

type FieldProps = InputHTMLAttributes<HTMLInputElement> & {
  id: string;
  label: string;
  error?: string | undefined;
  hint?: string | undefined;
};

/** Labelled input with its error rendered inline, right underneath. */
export function Field({
  id,
  label,
  error,
  hint,
  className,
  ...input
}: FieldProps) {
  const describedBy = error ? `${id}-error` : hint ? `${id}-hint` : undefined;
  return (
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor={id}
        className="text-sm font-medium text-neutral-800 dark:text-neutral-200"
      >
        {label}
      </label>
      <input
        id={id}
        aria-invalid={error ? true : undefined}
        aria-describedby={describedBy}
        className={[
          "rounded-lg border bg-white dark:bg-neutral-900 px-3 py-2 text-base text-neutral-900 dark:text-neutral-100 outline-none transition",
          "focus:ring-2 focus:ring-offset-0",
          error
            ? "border-red-400 dark:border-red-600 focus:border-red-500 dark:focus:border-red-400 focus:ring-red-200 dark:focus:ring-red-900"
            : "border-neutral-300 dark:border-neutral-600 focus:border-neutral-500 dark:focus:border-neutral-400 focus:ring-neutral-200 dark:focus:ring-neutral-700",
          className ?? "",
        ].join(" ")}
        {...input}
      />
      {error ? (
        <p
          id={`${id}-error`}
          role="alert"
          className="text-sm text-red-600 dark:text-red-400"
        >
          {error}
        </p>
      ) : hint ? (
        <p
          id={`${id}-hint`}
          className="text-sm text-neutral-500 dark:text-neutral-400"
        >
          {hint}
        </p>
      ) : null}
    </div>
  );
}
