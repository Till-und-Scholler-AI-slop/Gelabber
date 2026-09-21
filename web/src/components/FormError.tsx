/** Form-level error (wrong password, server unreachable, …). */
export function FormError({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <p
      role="alert"
      className="rounded-lg border border-red-200 dark:border-red-800 bg-red-50 dark:bg-red-950 px-3 py-2 text-sm text-red-700 dark:text-red-300"
    >
      {message}
    </p>
  );
}
