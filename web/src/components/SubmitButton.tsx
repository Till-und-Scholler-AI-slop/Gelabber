import type { ReactNode } from "react";

import { Button, type ButtonProps } from "./ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";

type SubmitButtonProps = Omit<
  ButtonProps,
  "type" | "pending" | "pendingLabel" | "variant"
> & {
  pending: boolean;
  pendingLabel: string;
  /** Seconds the server asked to wait; the button counts them down. */
  cooldown?: number;
  children: ReactNode;
};

/**
 * The form's primary action. Pending state is a label swap, not a spinner:
 * the request has a hard timeout, so the label always flips back — to
 * success or to an inline error.
 */
export function SubmitButton({
  cooldown = 0,
  disabled,
  children,
  ...props
}: SubmitButtonProps) {
  const t = useT(shell);
  return (
    <Button
      type="submit"
      variant="primary"
      disabled={disabled || cooldown > 0}
      {...props}
    >
      {cooldown > 0 ? t("submit.cooldown", { seconds: cooldown }) : children}
    </Button>
  );
}
