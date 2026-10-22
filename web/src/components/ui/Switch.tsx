import type { InputHTMLAttributes, ReactNode, Ref } from "react";

import "./ui.css";

type SwitchProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & {
  /** With a label the whole row is the control: label left, switch right. */
  label?: ReactNode;
  description?: ReactNode;
  ref?: Ref<HTMLInputElement>;
};

/** An on/off setting that applies at once. Without `label`, name it yourself. */
export function Switch({
  label,
  description,
  className,
  ...input
}: SwitchProps) {
  const control = (
    <input
      type="checkbox"
      role="switch"
      className={["ui-switch", label ? undefined : className]
        .filter(Boolean)
        .join(" ")}
      {...input}
    />
  );
  if (!label) return control;
  return (
    <label
      className={["ui-choice-row ui-choice-row--between", className]
        .filter(Boolean)
        .join(" ")}
    >
      <span className="ui-choice-text">
        {label}
        {description ? <small>{description}</small> : null}
      </span>
      {control}
    </label>
  );
}
