import { useToasts } from "./toasts.ts";

export function Toasts() {
  const toasts = useToasts((state) => state.toasts);
  const dismiss = useToasts((state) => state.dismiss);
  if (toasts.length === 0) return null;
  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex flex-col items-center gap-2 px-4"
    >
      {toasts.map((toast) => (
        <button
          key={toast.id}
          type="button"
          onClick={() => dismiss(toast.id)}
          className={[
            "pointer-events-auto rounded-lg px-4 py-2 text-sm shadow-lg",
            toast.tone === "error"
              ? "bg-red-600 text-white"
              : "bg-neutral-900 text-white",
          ].join(" ")}
        >
          {toast.message}
        </button>
      ))}
    </div>
  );
}
