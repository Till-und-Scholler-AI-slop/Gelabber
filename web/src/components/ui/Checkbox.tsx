import type { InputHTMLAttributes, ReactNode, Ref } from "react";

import "./ui.css";

type CheckboxProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & {
  label: ReactNode;
  description?: ReactNode;
  ref?: Ref<HTMLInputElement>;
};

/** A checkbox whose whole row, label included, is the target. */
export function Checkbox({
  label,
  description,
  className,
  ...input
}: CheckboxProps) {
  return (
    <label className={["ui-choice-row", className].filter(Boolean).join(" ")}>
      <input type="checkbox" className="ui-check" {...input} />
      <span className="ui-choice-text">
        {label}
        {description ? <small>{description}</small> : null}
      </span>
    </label>
  );
}
