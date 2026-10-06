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
    <section
      className="flex min-h-0 flex-1 flex-col"
      aria-label="Nachrichten suchen"
    >
      <form onSubmit={submit} className="flex flex-wrap gap-2 border-b p-3">
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
          className="min-w-0 flex-1 rounded border px-3 py-2"
          placeholder="In dieser Unterhaltung suchen"
        />
        <button
          type="submit"
          disabled={!input.trim()}
          className="rounded border px-3 py-2"
        >
          Suchen
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded border px-3 py-2"
        >
          Zurück zum Chat
        </button>
      </form>
      <div
        className="min-h-0 flex-1 overflow-y-auto px-4 py-3"
        aria-live="polite"
        aria-busy={query.isFetching || undefined}
      >
        {!q ? (
          <p className="text-sm">
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
          <article key={message.id} className="border-b py-3">
            <button
              type="button"
              ref={(node) => {
                if (node) resultButtons.current.set(message.id, node);
                else resultButtons.current.delete(message.id);
              }}
              onClick={() => setSelected(message.id)}
              aria-label={`Zur Nachricht von ${message.author.name}: ${message.content || "Datei"}`}
              className="w-full rounded p-2 text-left hover:bg-neutral-100 focus-visible:outline-2 dark:hover:bg-neutral-800"
            >
              <p className="text-sm">
                <strong>{message.author.name}</strong>{" "}
                <time dateTime={message.created_at}>
                  {new Date(message.created_at).toLocaleString("de-DE")}
                </time>
              </p>
              <p className="whitespace-pre-wrap break-words">
                {message.content}
              </p>
              <span className="text-xs underline">
                Nachricht im Kontext öffnen
              </span>
            </button>
          </article>
        ))}
        {query.hasNextPage ? (
          <button
            type="button"
            disabled={query.isFetchingNextPage}
            onClick={() => void query.fetchNextPage()}
            className="mt-3 rounded border px-3 py-2"
          >
            {query.isFetchingNextPage ? "Lädt…" : "Ältere Treffer"}
          </button>
        ) : null}
      </div>
    </section>
  );
}
