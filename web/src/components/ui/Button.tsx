import {
  useEffect,
  useState,
  type ButtonHTMLAttributes,
  type ReactNode,
  type Ref,
} from "react";

import { dialogInitialFocus } from "../dialogFocus.ts";
import {
  buttonClass,
  type ButtonSize,
  type ButtonVariant,
} from "./buttonClass.ts";
import { pendingFocus } from "./pendingFocus.ts";
import "./ui.css";

export type { ButtonSize, ButtonVariant };

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  /** One `primary` per view; `danger` and `danger-outline` for destructive actions. */
  variant?: ButtonVariant;
  /** `sm` 32px, `md` 36px, `lg` 44px; every size is 44px under a finger. */
  size?: ButtonSize;
  /**
   * While true the label reads `pendingLabel`, and the button is disabled
   * and `aria-busy`. A label swap, not a spinner: requests time out, so the
   * label always flips back.
   */
  pending?: boolean;
  pendingLabel?: ReactNode;
  /** Shown before the label. */
  icon?: ReactNode;
  /** Focus this button when its dialog opens. */
  initialFocus?: boolean;
  ref?: Ref<HTMLButtonElement>;
};

export function Button({
  variant = "secondary",
  size = "md",
  pending = false,
  pendingLabel,
  icon,
  initialFocus,
  className,
  children,
  disabled,
  type = "button",
  ref,
  ...rest
}: ButtonProps) {
  const focus = dialogInitialFocus(initialFocus);
  const [focusMemory] = useState(pendingFocus);
  useEffect(() => {
    if (!pending) focusMemory.settled();
  }, [pending, focusMemory]);
  return (
    <button
      type={type}
      {...rest}
      autoFocus={focus.autoFocus ?? rest.autoFocus}
      ref={
        focus.ref && ref
          ? (element: HTMLButtonElement | null) => {
              focus.ref(element);
              if (typeof ref === "function") return ref(element);
              ref.current = element;
            }
          : (focus.ref ?? ref)
      }
      onFocus={(event) => {
        focusMemory.focused(event.currentTarget);
        rest.onFocus?.(event);
      }}
      onBlur={(event) => {
        focusMemory.blurred(event.currentTarget);
        rest.onBlur?.(event);
      }}
      disabled={pending || disabled}
      aria-busy={pending || undefined}
      className={buttonClass({ variant, size, className })}
    >
      {icon}
      {pending && pendingLabel !== undefined ? pendingLabel : children}
    </button>
  );
}

/** Secondary action in dialogs ("Abbrechen", "Fertig"). */
export function GhostButton({
  tone = "neutral",
  ...props
}: Omit<ButtonProps, "variant"> & {
  tone?: "neutral" | "danger";
}) {
  return (
    <Button
      variant={tone === "danger" ? "danger-outline" : "ghost"}
      {...props}
    />
  );
}
