import { fieldDescription } from "./ui/fieldDescription.ts";
import { FormField } from "./ui/FormField.tsx";
import { Input, type InputProps } from "./ui/Input.tsx";

type FieldProps = Omit<InputProps, "invalid"> & {
  id: string;
  label: string;
  error?: string | undefined;
  hint?: string | undefined;
};

/** Labelled input with its error rendered inline, right underneath. */
export function Field({ id, label, error, hint, ...input }: FieldProps) {
  return (
    <FormField id={id} label={label} error={error} hint={hint}>
      <Input
        id={id}
        invalid={Boolean(error)}
        aria-describedby={fieldDescription(id, { error, hint })}
        {...input}
      />
    </FormField>
  );
}
