export function LoadError({
  message,
  pending = false,
  onRetry,
}: {
  message: string;
  pending?: boolean;
  onRetry: () => void;
}) {
  return (
    <div
      role="alert"
      className="flex flex-col items-start gap-3 rounded-lg border border-red-300 p-4 text-sm text-red-700 dark:border-red-800 dark:text-red-300"
    >
      <p>{message}</p>
      <button
        type="button"
        disabled={pending}
        onClick={onRetry}
        className="rounded border px-3 py-2 font-medium disabled:opacity-50"
      >
        {pending ? "Wird geladen…" : "Erneut versuchen"}
      </button>
    </div>
  );
}
