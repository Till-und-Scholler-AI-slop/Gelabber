import { useInfiniteQuery } from "@tanstack/react-query";
import {
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { searchMessages } from "./search.ts";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import type { Message } from "./types.ts";
import { MessageContextView } from "./MessageContext.tsx";

export function MessageSearch({
  channelId,
  onClose,
  renderMessage,
}: {
  channelId: string;
  onClose: () => void;
  renderMessage: (message: Message) => ReactNode;
}) {
  const userId = useUserId();
  const generation = scopeGeneration();
  const [input, setInput] = useState("");
  const [q, setQ] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const resultButtons = useRef(new Map<string, HTMLButtonElement>());
  const lastSelected = useRef<string | null>(null);
  useLayoutEffect(() => {
    if (!selected && lastSelected.current)
      resultButtons.current.get(lastSelected.current)?.focus();
    lastSelected.current = selected;
  }, [selected]);
  const query = useInfiniteQuery({
    queryKey: ["user", userId, generation, "message-search", channelId, q],
    queryFn: ({ pageParam, signal }) =>
      searchMessages(channelId, q, pageParam, signal),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => {
      const oldest = page.messages[0];
      return page.has_more && oldest
        ? `${oldest.created_at}|${oldest.id}`
        : undefined;
    },
    enabled: Boolean(userId && q),
    retry: false,
    staleTime: 0,
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const next = input.trim();
    if (q === next) void query.refetch();
    else setQ(next);
  };
  const rows =
    query.data?.pages.flatMap((page) => [...page.messages].reverse()) ?? [];
  if (selected)
    return (
      <MessageContextView
        key={selected}
        channelId={channelId}
        messageId={selected}
        onBack={() => setSelected(null)}
        onClose={onClose}
        renderMessage={renderMessage}
      />
    );
  return (
    <section className="lr-search" aria-label="Nachrichten suchen">
      <form onSubmit={submit} className="lr-search-form">
        <label className="sr-only" htmlFor={`search-${channelId}`}>
          In dieser Unterhaltung suchen
        </label>
        <input
          id={`search-${channelId}`}
          autoFocus
          type="search"
          value={input}
          maxLength={200}
          onChange={(event) => setInput(event.target.value)}
          placeholder="In dieser Unterhaltung suchen"
        />
        <button
          type="submit"
          disabled={!input.trim()}
          className="lr-search-submit"
        >
          Suchen
        </button>
        <button type="button" onClick={onClose} className="lr-search-close">
          Zurück zum Chat
        </button>
      </form>
      <div
        className="lr-search-results"
        aria-live="polite"
        aria-busy={query.isFetching || undefined}
      >
        {!q ? (
          <p className="lr-search-hint">
            Suche nach Wörtern oder einer Phrase in Anführungszeichen.
          </p>
        ) : null}
        {q && query.isPending ? <p>Suche läuft…</p> : null}
        {query.error ? (
          <div role="alert">
            <p>Die Suche konnte nicht geladen werden.</p>
            <button
              type="button"
              onClick={() =>
                query.isFetchNextPageError
                  ? void query.fetchNextPage()
                  : void query.refetch()
              }
            >
              Erneut laden
            </button>
          </div>
        ) : null}
        {q && query.isSuccess && rows.length === 0 ? (
          <p>Keine Nachrichten gefunden.</p>
        ) : null}
        {rows.map((message) => (
          <article key={message.id}>
            <button
              type="button"
              ref={(node) => {
                if (node) resultButtons.current.set(message.id, node);
                else resultButtons.current.delete(message.id);
              }}
              onClick={() => setSelected(message.id)}
              aria-label={`Zur Nachricht von ${message.author.name}: ${message.content || "Datei"}`}
              className="lr-search-hit"
            >
              <p>
                <strong>{message.author.name}</strong>{" "}
                <time dateTime={message.created_at}>
                  {new Date(message.created_at).toLocaleString("de-DE")}
                </time>
              </p>
              <p>{message.content}</p>
              <span>Nachricht im Kontext öffnen</span>
            </button>
          </article>
        ))}
        {query.hasNextPage ? (
          <button
            type="button"
            disabled={query.isFetchingNextPage}
            onClick={() => void query.fetchNextPage()}
            className="lr-button-secondary"
          >
            {query.isFetchingNextPage ? "Lädt…" : "Ältere Treffer"}
          </button>
        ) : null}
      </div>
    </section>
  );
}
