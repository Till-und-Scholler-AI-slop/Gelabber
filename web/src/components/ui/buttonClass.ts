export type ButtonVariant =
  "primary" | "secondary" | "ghost" | "danger" | "danger-outline";
export type ButtonSize = "sm" | "md" | "lg";

/** The classes of a button, for a link or summary that should look like one. */
export function buttonClass({
  variant = "secondary",
  size = "md",
  className,
}: {
  variant?: ButtonVariant;
  size?: ButtonSize;
  className?: string | undefined;
} = {}): string {
  return ["ui-button", `ui-button--${variant}`, `ui-button--${size}`, className]
    .filter(Boolean)
    .join(" ");
}
