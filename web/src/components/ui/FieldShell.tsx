import type { HTMLAttributes, LabelHTMLAttributes, Ref } from "react";

import "./ui.css";

type Common = {
  /** The danger frame, as `invalid` on a field gives it. */
  invalid?: boolean;
};

export type FieldShellProps = Common &
  (
    | ({
        as?: "div";
        ref?: Ref<HTMLDivElement>;
      } & HTMLAttributes<HTMLDivElement>)
    | ({
        as: "label";
        ref?: Ref<HTMLLabelElement>;
      } & LabelHTMLAttributes<HTMLLabelElement>)
  );

/**
 * One field frame around several parts: an icon, a `bare` Input or
 * Textarea, a badge, buttons (#561). The shell draws the border, the
 * background and the focus ring; the field inside draws none of them.
 */
export function FieldShell({
  invalid,
  className,
  as,
  ...rest
}: FieldShellProps) {
  const classes = ["ui-field-shell", className].filter(Boolean).join(" ");
  const state = invalid ? "" : undefined;
  if (as === "label") {
    const props = rest as LabelHTMLAttributes<HTMLLabelElement> & {
      ref?: Ref<HTMLLabelElement>;
    };
    return <label {...props} className={classes} data-invalid={state} />;
  }
  const props = rest as HTMLAttributes<HTMLDivElement> & {
    ref?: Ref<HTMLDivElement>;
  };
  return <div {...props} className={classes} data-invalid={state} />;
}
