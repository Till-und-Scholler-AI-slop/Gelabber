import type { ReactNode } from "react";

import { Button, type ButtonProps } from "./ui/Button.tsx";

type SubmitButtonProps = Omit<
  ButtonProps,
  "type" | "pending" | "pendingLabel" | "variant"
> & {
  pending: boolean;
  pendingLabel: string;
  children: ReactNode;
};

/**
 * The form's primary action. Pending state is a label swap, not a spinner:
 * the request has a hard timeout, so the label always flips back — to
 * success or to an inline error.
 */
export function SubmitButton(props: SubmitButtonProps) {
  return <Button type="submit" variant="primary" {...props} />;
}
