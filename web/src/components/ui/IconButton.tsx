import type { ButtonHTMLAttributes, ReactNode, Ref } from "react";

import "./ui.css";

export type IconButtonProps = Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "children" | "aria-label" | "aria-pressed"
> & {
  /** Accessible name; also the tooltip unless `title` says more. */
  label: string;
  icon: ReactNode;
  /** `xs` 28px, `sm` 32px, `md` 36px; 44px under a finger. */
  size?: "xs" | "sm" | "md";
  variant?: "ghost" | "secondary";
  /** For toggles: a fixed `label` plus this state, never a label swap. */
  pressed?: boolean;
  ref?: Ref<HTMLButtonElement>;
};

/** A button that shows only an icon, so it must be named. */
export function IconButton({
  label,
  icon,
  size = "md",
  variant = "ghost",
  pressed,
  title,
  className,
  type = "button",
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      aria-pressed={pressed}
      title={title ?? label}
      className={[
        "ui-icon-button",
        `ui-icon-button--${size}`,
        `ui-icon-button--${variant}`,
        className,
      ]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    >
      {icon}
    </button>
  );
}
