import { Button } from "./ui/Button.tsx";

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
    <div role="alert" className="ui-load-error">
      <p>{message}</p>
      <Button
        size="sm"
        pending={pending}
        pendingLabel="Wird geladen…"
        onClick={onRetry}
      >
        Erneut versuchen
      </Button>
    </div>
  );
}
