import type { ButtonHTMLAttributes, ReactNode } from "react";

type SubmitButtonProps = Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "type"
> & {
  pending: boolean;
  pendingLabel: string;
  children: ReactNode;
};

/**
 * Pending state is a label swap, not a spinner: the request has a hard
 * timeout, so the label always flips back — to success or to an inline error.
 */
export function SubmitButton({
  pending,
  pendingLabel,
  children,
  disabled,
  className,
  ...rest
}: SubmitButtonProps) {
  return (
    <button
      type="submit"
      disabled={pending || disabled}
      aria-busy={pending || undefined}
      className={[
        "rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition",
        "hover:bg-neutral-700 disabled:cursor-not-allowed disabled:opacity-60",
        className ?? "",
      ].join(" ")}
      {...rest}
    >
      {pending ? pendingLabel : children}
    </button>
  );
}
