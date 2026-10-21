import type { InputHTMLAttributes, ReactNode, Ref } from "react";

import "./ui.css";

type RadioCardProps = Omit<InputHTMLAttributes<HTMLInputElement>, "type"> & {
  /** Radios of one choice share a `name`; arrow keys move between them. */
  name: string;
  label: ReactNode;
  description?: ReactNode;
  ref?: Ref<HTMLInputElement>;
};

/** One option of a choice, drawn as a card; the checked one is highlighted. */
export function RadioCard({
  label,
  description,
  className,
  ...input
}: RadioCardProps) {
  return (
    <label className={["ui-radio-card", className].filter(Boolean).join(" ")}>
      <input type="radio" className="ui-check" {...input} />
      <span className="ui-choice-text">
        {label}
        {description ? <small>{description}</small> : null}
      </span>
    </label>
  );
}
