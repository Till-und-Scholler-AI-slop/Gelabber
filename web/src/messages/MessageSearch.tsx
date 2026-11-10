import { keepPreviousData, useInfiniteQuery } from "@tanstack/react-query";
import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { searchMessages } from "./search.ts";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import type { Message } from "./types.ts";
import { MessageContent } from "./MessageContent.tsx";
import { MessageContextView } from "./MessageContext.tsx";
import { readableMentions } from "./mentions.ts";
import { plainText } from "./markdown.ts";
import { searchTerms } from "./searchTerms.ts";
import { formatWhen } from "../components/messageTime.ts";
import {
  mentionRenderer,
  messagePing,
} from "../components/mentionRenderer.tsx";
import { useMentions } from "../components/mentionContext.ts";
import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";

/**
 * The hits for `q` in one conversation, shown where its history was. The
 * query is typed into the header's search field (#377); this only lists.
 */
export function MessageSearch({
  channelId,
  q,
  onClose,
  renderMessage,
}: {
  channelId: string;
  q: string;
  onClose: () => void;
  renderMessage: (message: Message) => ReactNode;
}) {
  const t = useT(chat);
  const userId = useUserId();
  const { members, me } = useMentions();
  const generation = scopeGeneration();
  // The hit opened in its context, until the query changes underneath it.
  const [opened, setOpened] = useState<{ q: string; id: string } | null>(null);
  const selected = opened?.q === q ? opened.id : null;
  const resultButtons = useRef(new Map<string, HTMLButtonElement>());
  const lastSelected = useRef<string | null>(null);
  useLayoutEffect(() => {
    if (!selected && lastSelected.current)
      resultButtons.current.get(lastSelected.current)?.focus();
    lastSelected.current = selected;
  }, [selected]);
  const query = useInfiniteQuery({
    queryKey: ["user", userId, generation, "message-search", channelId, q],
    // Each page keeps the query it answers, for the words it marks.
    queryFn: async ({ pageParam, signal }) => ({
      ...(await searchMessages(channelId, q, pageParam, signal)),
      q,
    }),
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
    // Typing on keeps the last hits instead of flashing "Suche läuft…".
    placeholderData: keepPreviousData,
  });
  const rows =
    query.data?.pages.flatMap((page) => [...page.messages].reverse()) ?? [];
  // Typing on keeps the last hits: they mark the words they were found by.
  const shownQ = query.data?.pages[0]?.q ?? q;
  const terms = useMemo(() => searchTerms(shownQ), [shownQ]);
  if (selected)
    return (
      <MessageContextView
        key={selected}
        channelId={channelId}
        messageId={selected}
        onBack={() => setOpened(null)}
        onClose={onClose}
        renderMessage={renderMessage}
      />
    );
  return (
    <section className="lr-search" aria-label={t("search.label")}>
      <p className="lr-search-summary">
        <span>{t("search.summary")}</span>
        <button type="button" onClick={onClose} className="lr-search-close">
          {t("pane.backToChat")}
        </button>
      </p>
      <div
        className="lr-search-results"
        aria-live="polite"
        aria-busy={query.isFetching || undefined}
      >
        {query.isPending ? <p>{t("search.running")}</p> : null}
        {query.error ? (
          <div role="alert">
            <p>{t("search.failed")}</p>
            <button
              type="button"
              onClick={() =>
                query.isFetchNextPageError
                  ? void query.fetchNextPage()
                  : void query.refetch()
              }
            >
              {t("list.reload")}
            </button>
          </div>
        ) : null}
        {query.isSuccess && rows.length === 0 ? (
          <p>{t("search.none")}</p>
        ) : null}
        {rows.map((message) => (
          <article key={message.id}>
            <button
              type="button"
              ref={(node) => {
                if (node) resultButtons.current.set(message.id, node);
                else resultButtons.current.delete(message.id);
              }}
              onClick={() => setOpened({ q, id: message.id })}
              aria-label={t("search.hitLabel", {
                name: message.author.name,
                text:
                  readableMentions(plainText(message.content), members) ||
                  t("notify.file"),
              })}
              className="lr-search-hit"
            >
              <p>
                <strong>{message.author.name}</strong>{" "}
                <time dateTime={message.created_at}>
                  {formatWhen(message.created_at)}
                </time>
              </p>
              <p>
                <MessageContent
                  content={message.content}
                  renderMention={mentionRenderer(
                    members,
                    me,
                    message.content,
                    messagePing(message),
                  )}
                  interactive={false}
                  highlight={terms}
                />
              </p>
              <span>{t("search.openInContext")}</span>
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
            {t(
              query.isFetchingNextPage ? "search.loadingMore" : "search.older",
            )}
          </button>
        ) : null}
      </div>
    </section>
  );
}
