import type { HTMLAttributes } from "react";

import "./ui.css";

type BadgeProps = HTMLAttributes<HTMLSpanElement> & {
  /** `count` is the round unread/mention pill; the others are 11px caps tags. */
  tone?: "neutral" | "accent" | "success" | "warning" | "danger" | "count";
};

export function Badge({ tone = "neutral", className, ...rest }: BadgeProps) {
  return (
    <span
      className={["ui-badge", `ui-badge--${tone}`, className]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    />
  );
}
