import { useState, type ReactNode } from "react";

import { EyeIcon, EyeOffIcon, WarningIcon } from "../Icons.tsx";
import { fieldDescription } from "./fieldDescription.ts";
import { FormField } from "./FormField.tsx";
import { IconButton } from "./IconButton.tsx";
import { Input, type InputProps } from "./Input.tsx";
import { passwordToggle } from "./passwordToggle.ts";

type PasswordFieldProps = Omit<InputProps, "type" | "id" | "invalid"> & {
  id: string;
  label: ReactNode;
  error?: string | undefined;
  hint?: ReactNode;
};

/**
 * Password input with a show/hide toggle and a Caps Lock warning. The
 * toggle keeps one name, "Passwort anzeigen", and reports its state with
 * `aria-pressed`.
 */
export function PasswordField({
  id,
  label,
  error,
  hint,
  className,
  onKeyDown,
  onKeyUp,
  onBlur,
  ...input
}: PasswordFieldProps) {
  const [shown, setShown] = useState(false);
  const [capsLock, setCapsLock] = useState(false);
  const toggle = passwordToggle(shown);
  const capsId = `${id}-caps`;
  return (
    <FormField id={id} label={label} error={error} hint={hint}>
      <div className="ui-password">
        <Input
          id={id}
          type={toggle.type}
          invalid={Boolean(error)}
          aria-describedby={fieldDescription(id, {
            error,
            hint,
            extra: capsLock ? capsId : undefined,
          })}
          className={className}
          onKeyDown={(event) => {
            setCapsLock(event.getModifierState("CapsLock"));
            onKeyDown?.(event);
          }}
          onKeyUp={(event) => {
            setCapsLock(event.getModifierState("CapsLock"));
            onKeyUp?.(event);
          }}
          onBlur={(event) => {
            setCapsLock(false);
            onBlur?.(event);
          }}
          {...input}
        />
        <IconButton
          className="ui-password__toggle"
          label="Passwort anzeigen"
          title={toggle.title}
          pressed={toggle.pressed}
          icon={shown ? <EyeOffIcon size={18} /> : <EyeIcon size={18} />}
          onClick={() => setShown((value) => !value)}
        />
      </div>
      {capsLock ? (
        <p id={capsId} className="ui-caps-hint">
          <WarningIcon size={14} />
          Feststelltaste ist an
        </p>
      ) : null}
    </FormField>
  );
}
