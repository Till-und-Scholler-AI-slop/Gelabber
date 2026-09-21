import { useNavigate } from "@tanstack/react-router";

import { useMessageToasts } from "../messages/toasts.ts";

export function MessageToasts() {
  const toasts = useMessageToasts((state) => state.toasts);
  const dismiss = useMessageToasts((state) => state.dismiss);
  const navigate = useNavigate();
  if (toasts.length === 0) return null;

  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed top-16 right-4 z-50 flex w-[min(100%-2rem,20rem)] flex-col gap-2"
    >
      {toasts.map((toast) => {
        const title =
          toast.count > 1
            ? `${toast.channelLabel} · ${toast.count} neu`
            : toast.channelLabel;
        return (
          <button
            key={toast.id}
            type="button"
            onClick={() => {
              dismiss(toast.id);
              if (toast.dm) {
                void navigate({
                  to: "/d/$channelId",
                  params: { channelId: toast.channelId },
                });
                return;
              }
              void navigate({
                to: "/s/$serverId/c/$channelId",
                params: {
                  serverId: toast.serverId,
                  channelId: toast.channelId,
                },
              });
            }}
            className="pointer-events-auto rounded-xl border border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-left shadow-lg transition hover:bg-neutral-50 dark:hover:bg-neutral-800"
          >
            <p className="truncate text-xs font-medium text-neutral-500 dark:text-neutral-400">
              {title}
            </p>
            <p className="truncate text-sm font-semibold text-neutral-900 dark:text-neutral-100">
              {toast.author}
            </p>
            <p className="truncate text-sm text-neutral-600 dark:text-neutral-400">
              {toast.preview}
            </p>
          </button>
        );
      })}
    </div>
  );
}
