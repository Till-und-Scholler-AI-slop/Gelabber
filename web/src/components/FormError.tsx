import "./ui/ui.css";

/**
 * Form-level error (wrong password, server unreachable, …): the one alert a
 * failed submit announces. Field errors are shown at their fields, quietly.
 */
export function FormError({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <p role="alert" className="ui-form-error">
      {message}
    </p>
  );
}
