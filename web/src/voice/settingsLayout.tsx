// The Voice & Video forms' layout: groups under a small heading, and rows
// with the setting's name on the left and its control on the right,
// stacked where the form is narrow (quality.css).

import { useId, type CSSProperties, type ReactNode } from "react";

import { Switch } from "../components/ui/Switch.tsx";

export function SettingsGroup({
  title,
  badge,
  children,
}: {
  title: string;
  /** The group's current choice, at a glance. */
  badge?: ReactNode;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <section className="media-group" aria-labelledby={id}>
      <header className="media-group-head">
        <h3 id={id}>{title}</h3>
        {badge ? <span>{badge}</span> : null}
      </header>
      {children}
    </section>
  );
}

/** One setting. With `htmlFor` the name labels that control; without, it
 * names the row as a group of controls. */
export function SettingRow({
  label,
  htmlFor,
  children,
}: {
  label: ReactNode;
  htmlFor?: string;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <div
      className="media-row"
      role={htmlFor ? undefined : "group"}
      aria-labelledby={htmlFor ? undefined : id}
    >
      {htmlFor ? (
        <label htmlFor={htmlFor} className="media-row-label">
          {label}
        </label>
      ) : (
        <span id={id} className="media-row-label">
          {label}
        </span>
      )}
      <div className="media-row-control">{children}</div>
    </div>
  );
}

/** A switch on the right of its explanation; the row's name labels it. */
export function SwitchLine({
  id,
  checked,
  onChange,
  disabled,
  children,
}: {
  id: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="media-switch-line">
      <p id={`${id}-hint`} className="media-hint">
        {children}
      </p>
      <Switch
        id={id}
        checked={checked}
        disabled={disabled}
        aria-describedby={`${id}-hint`}
        onChange={(event) => onChange(event.target.checked)}
      />
    </div>
  );
}

/** A range with its value beside it; a label elsewhere names it. */
export function Slider({
  id,
  min,
  max,
  value,
  suffix,
  step,
  onChange,
  hint,
  scale,
  disabled,
}: {
  id: string;
  min: number;
  max: number;
  value: number;
  suffix: string;
  step?: number;
  onChange: (value: number) => void;
  hint?: string;
  /** Labels under the start, middle and end of the track. */
  scale?: readonly [string, string, string];
  disabled?: boolean;
}) {
  return (
    <div className="media-slider">
      <input
        id={id}
        type="range"
        step={step}
        min={min}
        max={max}
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(Number(event.target.value))}
        style={
          {
            "--lr-range-value": `${Math.round(((value - min) / (max - min)) * 100)}%`,
          } as CSSProperties
        }
        className="lr-range"
      />
      <output htmlFor={id}>{suffix}</output>
      {scale ? (
        <div aria-hidden="true" className="media-slider-scale">
          {scale.map((mark) => (
            <span key={mark}>{mark}</span>
          ))}
        </div>
      ) : null}
      {hint ? <p className="media-hint media-slider-hint">{hint}</p> : null}
    </div>
  );
}
