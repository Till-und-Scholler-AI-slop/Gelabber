import { useQuery } from "@tanstack/react-query";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { ApiError } from "../api/client.ts";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import { messageContext } from "./search.ts";
import type { Message } from "./types.ts";

export function MessageContextView({
  channelId,
  messageId,
  onBack,
  onClose,
  renderMessage,
}: {
  channelId: string;
  messageId: string;
  onBack: () => void;
  onClose: () => void;
  renderMessage: (message: Message) => ReactNode;
}) {
  const userId = useUserId(),
    generation = scopeGeneration();
  const target = useRef<HTMLElement | null>(null);
  const focused = useRef(false);
  const query = useQuery({
    queryKey: [
      "user",
      userId,
      generation,
      "message-context",
      channelId,
      messageId,
    ],
    queryFn: ({ signal }) => messageContext(channelId, messageId, signal),
    enabled: Boolean(userId),
    staleTime: 0,
    retry: false,
  });
  useLayoutEffect(() => {
    if (!query.error && target.current && !focused.current) {
      focused.current = true;
      target.current.scrollIntoView({ block: "center" });
      target.current.focus({ preventScroll: true });
    }
  }, [query.data, query.error]);
  const missing =
    query.error instanceof ApiError &&
    ["not_found", "forbidden"].includes(query.error.code);
  return (
    <section
      className="flex min-h-0 flex-1 flex-col"
      aria-label="Nachrichtenkontext"
    >
      <div className="flex flex-wrap gap-2 border-b p-3">
        <button
          type="button"
          onClick={onBack}
          className="rounded border px-3 py-2"
        >
          Zurück zu den Treffern
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded border px-3 py-2"
        >
          Zurück zum Chat
        </button>
      </div>
      <div
        className="min-h-0 flex-1 overflow-y-auto py-3"
        aria-busy={query.isFetching || undefined}
      >
        {query.isPending ? (
          <p role="status" className="px-4">
            Nachrichtenkontext wird geladen…
          </p>
        ) : null}
        {query.error ? (
          <div role="alert" className="px-4">
            <p>
              {missing
                ? "Diese Nachricht ist nicht mehr verfügbar oder du hast keinen Zugriff mehr."
                : "Der Nachrichtenkontext konnte nicht geladen werden."}
            </p>
            <button
              type="button"
              onClick={() => void query.refetch()}
              disabled={query.isFetching}
              className="mt-2 rounded border px-3 py-2"
            >
              Erneut laden
            </button>
          </div>
        ) : (
          query.data?.messages.map((message) => (
            <article
              key={message.id}
              data-message-id={message.id}
              ref={message.id === messageId ? target : undefined}
              tabIndex={message.id === messageId ? -1 : undefined}
              aria-label={
                message.id === messageId
                  ? `Gefundene Nachricht von ${message.author.name}`
                  : undefined
              }
              className={
                message.id === messageId
                  ? "mx-1 rounded border-2 outline-none"
                  : ""
              }
              style={
                message.id === messageId
                  ? {
                      borderColor: "var(--lr-accent)",
                      background:
                        "color-mix(in srgb, var(--lr-accent) 12%, transparent)",
                    }
                  : undefined
              }
            >
              {renderMessage(message)}
            </article>
          ))
        )}
      </div>
    </section>
  );
}
