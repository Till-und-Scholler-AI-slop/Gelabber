// Text-channel body: virtualised history, optimistic composer, own
// edit/delete. A send writes the pending overlay first, so the row is
// painted in the same frame; the server answer swaps the `tmp:` id, an
// error retains an independently recoverable send attempt.

import { anchoredScrollTop } from "./scrollAnchor.ts";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  endDistance,
  scrollEndIntent,
  type ScrollPosition,
} from "../messages/scrollPosition.ts";
import { MessageSearch } from "../messages/MessageSearch.tsx";
import { MessageContextView } from "../messages/MessageContext.tsx";
import {
  readBoundary,
  useMarkManyRead,
  useMarkRead,
  useReadState,
} from "../messages/readState.ts";
import { useSession } from "../auth/session.ts";
import { useLocalDay } from "../lib/localDay.ts";
import { arrivals, type Arrival } from "../messages/arrivals.ts";
import {
  arrivedSince,
  confirmedPendingIds,
  isContinued,
  UNREAD_SEEK_PAGES,
  unreadAnchor,
  visibleMessages,
  type ReadMark,
} from "../messages/pages.ts";
import {
  nonePending,
  displayPending,
  type SendAttempt,
  removePending,
  usePendingMessages,
} from "../messages/pending.ts";
import {
  useDeleteMessage,
  useEditMessage,
  useMessages,
  useSendMessage,
} from "../messages/queries.ts";
import { isAudioType, isImageType, isVideoType } from "../messages/rules.ts";
import type { Message } from "../messages/types.ts";
import { asAttachmentList, asGif } from "../messages/types.ts";
import { gifBox } from "../gifs/api.ts";
import { Avatar } from "./Avatar.tsx";
import { confirmAction } from "./confirm.ts";
import { ImageLightbox, type LightboxImage } from "./ImageLightbox.tsx";
import { ArrowDownIcon, HashIcon, SearchIcon } from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { SkeletonRows } from "./ui/Skeleton.tsx";
import { useAfterSkeleton } from "./ui/useAfterSkeleton.ts";
import { OpenImage } from "./openImage.ts";
import { MentionContext } from "./mentionContext.ts";
import { type MentionMember } from "../messages/mentions.ts";
import { NotificationBell } from "./NotificationSettingsDialog.tsx";
import type { NotificationTarget } from "./notificationTarget.ts";
import { useMuteClock } from "../messages/notificationSettings.ts";
import { isMuted } from "../messages/notifyLevel.ts";
import "./chat.css";
import { Composer } from "./Composer.tsx";
import { MessageRow } from "./MessageRow.tsx";
import {
  AttemptActionsContext,
  JumpTo,
  type AttemptActions,
  type ComposerHandle,
} from "./messagePaneContext.ts";
import { dayBreak, dayLabel, formatWhen } from "./messageTime.ts";

const NO_MEMBERS: readonly MentionMember[] = [];

export function MessagePane({
  title,
  notice,
  actions,
  channelId,
  channelName,
  canSend,
  canSendFiles = false,
  canModerate = false,
  canMentionEveryone = false,
  mention = "#",
  members = NO_MEMBERS,
  notify,
  peer,
  footer,
  onDraftChange,
  onDraftStop,
}: {
  /** Header content left of the search button: icon and channel name. */
  title: ReactNode;
  /** Optional strip below the header, e.g. the live hint. */
  notice?: ReactNode;
  /** Extra header buttons right of the search, e.g. the member list toggle. */
  actions?: ReactNode;
  channelId: string;
  channelName: string;
  canSend: boolean;
  canSendFiles?: boolean;
  canModerate?: boolean;
  /** Offer `@everyone` / `@here` in a server channel's suggestions. */
  canMentionEveryone?: boolean;
  /** `#` for a server channel, `@` for a DM. */
  mention?: "#" | "@";
  /** Who can be mentioned here: the server's members, a DM's two people. */
  members?: readonly MentionMember[];
  /** The bell in the header: notification settings of this conversation. */
  notify?: NotificationTarget;
  /** A DM's other person, shown where its history begins. */
  peer?: { name: string; avatar_url: string | null };
  footer?: ReactNode;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const user = useSession((s) => s.user);
  const query = useMessages(channelId, true);
  const searchScope = `${user?.id ?? "anonymous"}:${channelId}`;
  const [searchingScope, setSearchingScope] = useState<string | null>(null);
  const searching = searchingScope === searchScope;
  // Per channel and account, like the search: a switch closes it.
  const [lightbox, setLightbox] = useState<{
    scope: string;
    image: LightboxImage;
    /** Its message was in the list; gone from it again means deleted. */
    listed?: boolean;
  } | null>(null);
  const openImage = useCallback(
    (image: LightboxImage) => setLightbox({ scope: searchScope, image }),
    [searchScope],
  );
  const panesRef = useRef<HTMLDivElement>(null);
  // The opening row is gone: the topmost row in view, so a keyboard user
  // stays in the list instead of on <body>.
  const focusList = useCallback(() => {
    const log = panesRef.current?.querySelector<HTMLElement>('[role="log"]');
    if (!log) return;
    const top = log.getBoundingClientRect().top;
    const row = [...log.querySelectorAll<HTMLElement>(".lr-message-row")].find(
      (candidate) => candidate.getBoundingClientRect().bottom > top,
    );
    row?.focus({ preventScroll: true });
  }, []);
  // A reply's original: scrolled to in the list, or shown in its context.
  const [focusRequest, setFocusRequest] = useState<{
    id: string;
    n: number;
  } | null>(null);
  const [contextTarget, setContextTarget] = useState<{
    scope: string;
    id: string;
  } | null>(null);
  const showingContext = contextTarget?.scope === searchScope;
  const searchButton = useRef<HTMLButtonElement>(null);
  const restoreSearchFocus = useRef(false);
  const closeSearch = useCallback(() => {
    restoreSearchFocus.current = true;
    setSearchingScope(null);
  }, []);
  useLayoutEffect(() => {
    if (!searching && restoreSearchFocus.current) {
      restoreSearchFocus.current = false;
      searchButton.current?.focus({ preventScroll: true });
    }
  }, [searching]);
  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (
        (event.ctrlKey || event.metaKey) &&
        !event.shiftKey &&
        !event.altKey &&
        event.key.toLowerCase() === "k"
      ) {
        event.preventDefault();
        setSearchingScope(searchScope);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [searchScope]);
  // Esc marks this conversation read, Shift+Esc its whole server (#245).
  // Only an Esc nothing else used: a dialog, a menu, the search, an edit
  // or the mention list closing takes it first.
  const markManyRead = useMarkManyRead();
  const markReadNow = (server = false) => {
    const serverId = notify?.serverId;
    void markManyRead(
      server && serverId ? { server: serverId } : { channel: channelId },
    ).catch(() => {});
  };
  const markByKey = useEffectEvent((server: boolean) => markReadNow(server));
  useEffect(() => {
    if (searching) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (
        event.key !== "Escape" ||
        event.defaultPrevented ||
        event.isComposing ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        document.querySelector("dialog[open], details[open]")
      )
        return;
      markByKey(event.shiftKey);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [searching]);
  // Where "Neu" goes: the read position when the channel was opened, kept
  // while reading so the divider does not jump.
  const { data: readRows, isError: readStateFailed } = useReadState();
  const readRow = readRows?.find((row) => row.channel_id === channelId);
  const [newAfter, setNewAfter] = useState<ReadMark | null | undefined>(
    undefined,
  );
  // How many were unread then, for the bar; null once they are read.
  const [unreadCount, setUnreadCount] = useState<number | null>(null);
  if (newAfter === undefined && (readRows || readStateFailed)) {
    const id =
      readRow && readRow.unread_count > 0 ? readRow.read_message_id : null;
    setNewAfter(id ? { id, order: readRow?.read_order } : null);
    setUnreadCount(id ? (readRow?.unread_count ?? null) : null);
  }
  if (unreadCount !== null && readRow?.unread_count === 0) setUnreadCount(null);
  // Whether the list opened at the first unread row rather than the end.
  const [openedAtUnread, setOpenedAtUnread] = useState(false);
  const [unreadJump, setUnreadJump] = useState(0);
  const contextEdit = useEditMessage(channelId);
  const contextRemove = useDeleteMessage(channelId);
  const [atLatest, setAtLatest] = useState(false);
  const [latestRequest, setLatestRequest] = useState(0);
  const toLatest = useCallback(
    () => setLatestRequest((value) => value + 1),
    [],
  );
  const attempts = usePendingMessages((s) => s.attempts);
  const channelAttempts = useMemo(
    () =>
      Object.values(attempts).filter(
        (attempt) => attempt.channelId === channelId,
      ),
    [attempts, channelId],
  );
  const attemptsById = useMemo(
    () => new Map(channelAttempts.map((attempt) => [attempt.id, attempt])),
    [channelAttempts],
  );
  const author = useMemo(
    () =>
      user
        ? { id: user.id, name: user.name, avatar_url: user.avatar_url }
        : null,
    [user],
  );
  const resend = useSendMessage(
    channelId,
    author ?? { id: "", name: "", avatar_url: null },
  ).mutate;
  const composer = useRef<ComposerHandle>(null);
  const attemptActions = useMemo<AttemptActions>(
    () => ({
      retry: (attempt) =>
        resend({ content: attempt.content, attemptId: attempt.id }),
      edit: (attempt) => composer.current?.takeBack(attempt),
      canSendFiles,
    }),
    [resend, canSendFiles],
  );
  const pending = usePendingMessages(
    (s) => s.byChannel[channelId] ?? nonePending,
  );
  const items = useMemo(
    () => visibleMessages(query.data?.pages ?? [], pending),
    [query.data?.pages, pending],
  );
  const jumpTo = useCallback(
    (id: string) => {
      setSearchingScope(null);
      if (items.some((message) => message.id === id)) {
        setContextTarget(null);
        setFocusRequest((previous) => ({ id, n: (previous?.n ?? 0) + 1 }));
      } else setContextTarget({ scope: searchScope, id });
    },
    [items, searchScope],
  );
  const overlay = searching || showingContext;
  // "Anzeigen" under the composer: to the row that was not sent, and onto
  // its first action once the list rendered it there.
  const showAttempt = useCallback(
    (id: string) => {
      jumpTo(id);
      const index = items.findIndex((message) => message.id === id);
      let frames = 10;
      const focusAction = () => {
        const action = panesRef.current?.querySelector<HTMLElement>(
          `[role="log"] [data-index="${index}"] .lr-message-failure button:not(:disabled)`,
        );
        if (action) action.focus({ preventScroll: true });
        else if (--frames > 0) requestAnimationFrame(focusAction);
      };
      if (index >= 0) requestAnimationFrame(focusAction);
    },
    [jumpTo, items],
  );

  // The newest row when the reader left the end: what others wrote since
  // is counted on the way back.
  const [awayFrom, setAwayFrom] = useState<string | null>(null);
  const newestId = items.at(-1)?.id;
  if (atLatest && awayFrom !== null) setAwayFrom(null);
  else if (!atLatest && awayFrom === null && newestId) setAwayFrom(newestId);
  const arrived = awayFrom ? arrivedSince(items, awayFrom, user?.id) : 0;
  const opensAt = unreadAnchor(
    items,
    newAfter ?? null,
    Boolean(query.hasNextPage),
  );
  const firstUnread = opensAt.kind === "row" ? items[opensAt.index] : undefined;

  // Close the picture with its message when that is deleted. A result of
  // the search may not be in the loaded list: only "was there, now gone".
  if (lightbox) {
    const listed = items.some((message) =>
      (message.attachments ?? []).some(
        (attachment) => attachment.id === lightbox.image.id,
      ),
    );
    if (listed && !lightbox.listed) setLightbox({ ...lightbox, listed });
    else if (!listed && lightbox.listed) setLightbox(null);
  }

  const latest = readBoundary(items);
  const read = useMarkRead(
    channelId,
    latest?.id,
    atLatest && !searching,
    !query.isFetching && (!query.error || query.isFetchNextPageError),
  );

  useEffect(() => displayPending(channelId), [channelId]);
  useEffect(() => {
    const pages = query.data?.pages ?? [];
    for (const id of confirmedPendingIds(pages, pending)) {
      removePending(channelId, id);
    }
  }, [channelId, pending, query.data?.pages]);
  const fetchNextPage = query.fetchNextPage;
  const hasNextPage = query.hasNextPage;
  const isFetchingNextPage = query.isFetchingNextPage;
  const onLoadOlder = useCallback(() => {
    if (hasNextPage && !isFetchingNextPage && !query.isFetchNextPageError)
      void fetchNextPage();
  }, [
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
    query.isFetchNextPageError,
  ]);
  const muteNow = useMuteClock();
  const muted = isMuted(readRow, muteNow);
  const mentions = useMemo(
    () => ({
      members,
      me: user?.id,
      keywords: mention === "#" && canMentionEveryone,
    }),
    [members, user?.id, mention, canMentionEveryone],
  );

  return (
    <MentionContext.Provider value={mentions}>
      <div className="lr-message-pane flex min-h-0 flex-1 flex-col">
        <header className="lr-channel-header">
          {title}
          <button
            ref={searchButton}
            type="button"
            onClick={() => setSearchingScope(searching ? null : searchScope)}
            className="lr-header-search"
            aria-expanded={searching}
            aria-label="Nachrichten suchen"
            title="Nachrichten suchen"
          >
            <SearchIcon size={16} />
            <span>Suchen</span>
            <kbd>Strg K</kbd>
          </button>
          {notify ? <NotificationBell target={notify} muted={muted} /> : null}
          {actions}
        </header>
        {notice}
        {read.error ? (
          <p className="lr-inline-notice" role="status">
            <span>Lesestatus nicht gespeichert.</span>
            <button type="button" onClick={read.retry}>
              Erneut versuchen
            </button>
          </p>
        ) : null}
        <OpenImage.Provider value={openImage}>
          <JumpTo.Provider value={jumpTo}>
            <AttemptActionsContext.Provider value={attemptActions}>
              <div
                ref={panesRef}
                className="relative flex min-h-0 flex-1 flex-col"
              >
                {unreadCount !== null && openedAtUnread && !overlay ? (
                  <section
                    className="lr-unread-bar"
                    aria-label="Ungelesene Nachrichten"
                  >
                    <p>
                      <strong>
                        {unreadCount === 1
                          ? "1 neue Nachricht"
                          : // The server counts up to 100.
                            `${unreadCount > 99 ? "99+" : unreadCount} neue Nachrichten`}
                      </strong>
                      {firstUnread
                        ? ` seit ${formatWhen(firstUnread.created_at)}`
                        : null}
                    </p>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setUnreadJump((value) => value + 1)}
                    >
                      Springen
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-keyshortcuts="Escape"
                      onClick={() => {
                        markReadNow();
                        // The bar goes with the unread messages: stay in the
                        // list rather than on <body>.
                        focusList();
                      }}
                    >
                      Als gelesen markieren
                      <kbd aria-hidden="true">Esc</kbd>
                    </Button>
                  </section>
                ) : null}
                <div
                  className={`flex min-h-0 flex-1 flex-col ${overlay ? "invisible" : ""}`}
                  inert={overlay}
                  aria-hidden={overlay || undefined}
                >
                  <MessageList
                    key={searchScope}
                    active={!overlay}
                    focusRequest={focusRequest}
                    channelId={channelId}
                    items={items}
                    onAtLatest={setAtLatest}
                    latestRequest={latestRequest}
                    meId={user?.id}
                    newAfter={newAfter}
                    unreadJump={unreadJump}
                    onOpened={setOpenedAtUnread}
                    attempts={attemptsById}
                    historyStart={
                      <HistoryStart
                        mention={mention}
                        channelName={channelName}
                        peer={peer}
                      />
                    }
                    canModerate={canModerate}
                    canSend={canSend}
                    hasOlder={Boolean(hasNextPage)}
                    loadingOlder={isFetchingNextPage}
                    onLoadOlder={onLoadOlder}
                    ready={!query.isPending}
                    loadError={
                      query.error
                        ? query.isFetchNextPageError
                          ? "paging"
                          : "history"
                        : null
                    }
                    onRetry={() => {
                      if (query.isFetchNextPageError) void fetchNextPage();
                      else void query.refetch();
                    }}
                  />
                </div>
                {searching ? (
                  <div className="lr-search-overlay absolute inset-0 flex min-h-0 flex-col">
                    <MessageSearch
                      key={searchScope}
                      channelId={channelId}
                      onClose={closeSearch}
                      renderMessage={(message) => (
                        <MessageRow
                          message={message}
                          continued={false}
                          canSend={canSend}
                          mine={message.author.id === user?.id}
                          canDelete={
                            message.author.id === user?.id || canModerate
                          }
                          onEdit={(content, failed) =>
                            contextEdit.mutate(
                              { id: message.id, content },
                              { onError: failed },
                            )
                          }
                          onDelete={async (skipConfirm) => {
                            if (
                              skipConfirm ||
                              (await confirmDelete(message, user?.id))
                            )
                              contextRemove.mutate(message.id);
                          }}
                        />
                      )}
                    />
                  </div>
                ) : null}
                {showingContext && !searching ? (
                  <div className="lr-search-overlay absolute inset-0 flex min-h-0 flex-col">
                    <MessageContextView
                      key={contextTarget.id}
                      channelId={channelId}
                      messageId={contextTarget.id}
                      closeLabel="Zurück zum Chat"
                      onClose={() => setContextTarget(null)}
                      renderMessage={(message) => (
                        <MessageRow
                          message={message}
                          continued={false}
                          canSend={canSend}
                          mine={message.author.id === user?.id}
                          canDelete={
                            message.author.id === user?.id || canModerate
                          }
                          onEdit={(content, failed) =>
                            contextEdit.mutate(
                              { id: message.id, content },
                              { onError: failed },
                            )
                          }
                          onDelete={async (skipConfirm) => {
                            if (
                              skipConfirm ||
                              (await confirmDelete(message, user?.id))
                            )
                              contextRemove.mutate(message.id);
                          }}
                        />
                      )}
                    />
                  </div>
                ) : null}
                {!overlay && !atLatest && items.length > 0 ? (
                  <button
                    type="button"
                    onClick={toLatest}
                    className={`lr-jump-latest${arrived > 0 ? " has-news" : ""}`}
                  >
                    <ArrowDownIcon size={15} />
                    {arrived === 0
                      ? "Zu den neuesten Nachrichten"
                      : arrived === 1
                        ? "1 neue Nachricht"
                        : `${arrived} neue Nachrichten`}
                  </button>
                ) : null}
              </div>
            </AttemptActionsContext.Provider>
          </JumpTo.Provider>
        </OpenImage.Provider>
        {lightbox?.scope === searchScope ? (
          <ImageLightbox
            key={lightbox.image.id}
            image={lightbox.image}
            onClose={() => setLightbox(null)}
            fallbackFocus={focusList}
          />
        ) : null}
        {footer}
        <Composer
          key={user?.id ?? "anonymous"}
          handle={composer}
          channelId={channelId}
          channelName={channelName}
          canSend={canSend}
          canSendFiles={canSendFiles}
          mention={mention}
          author={author}
          attempts={channelAttempts}
          onSent={toLatest}
          onShowAttempt={showAttempt}
          onDraftChange={onDraftChange}
          onDraftStop={onDraftStop}
        />
      </div>
    </MentionContext.Provider>
  );
}

function MessageList({
  active,
  channelId,
  items,
  meId,
  newAfter,
  canModerate,
  canSend,
  hasOlder,
  loadingOlder,
  onLoadOlder,
  ready,
  loadError,
  onRetry,
  onAtLatest,
  latestRequest,
  focusRequest,
  unreadJump,
  onOpened,
  attempts,
  historyStart,
}: {
  active: boolean;
  channelId: string;
  items: Message[];
  meId: string | undefined;
  /** Last read message when the channel opened; "Neu" goes after it.
   *  `undefined` while the read state is not known yet. */
  newAfter: ReadMark | null | undefined;
  canModerate: boolean;
  canSend: boolean;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  ready: boolean;
  loadError: "history" | "paging" | null;
  onRetry: () => void;
  onAtLatest: (value: boolean) => void;
  latestRequest: number;
  /** Scroll to this loaded row and highlight it (a reply's original). */
  focusRequest: { id: string; n: number } | null;
  /** Bumped by "Springen": back to the first unread row. */
  unreadJump: number;
  /** Whether the list opened at the first unread row, once it decided. */
  onOpened: (atUnread: boolean) => void;
  /** Send attempts by the `tmp:` id of their row. */
  attempts: ReadonlyMap<string, SendAttempt>;
  /** Above the oldest message once there is nothing older. */
  historyStart: ReactNode;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const stickToBottom = useRef(true);
  const contentRef = useRef<HTMLDivElement>(null);
  const lastPosition = useRef<ScrollPosition | null>(null);
  const touchY = useRef<number | null>(null);
  const lastRequest = useRef(latestRequest);
  // Older history was asked for; its rows are prepended above `pin`.
  const olderAnchor = useRef(false);
  const lastCount = useRef(0);
  const lastFirst = useRef<string | undefined>(undefined);
  // The rows the virtualizer measured, as committed (not as rendering).
  const itemsRef = useRef(items);
  const pin = useRef<{ id: string; offset: number } | null>(null);
  // The row a prepend or trim must keep in place, held until the reader
  // scrolls: the new rows above it are measured (and grow) in later commits.
  // Not `pin`, which every scroll and layout pass rewrites.
  const hold = useRef<{ id: string; offset: number } | null>(null);
  // Where the held anchor put the list; a scroll elsewhere is the reader's.
  const heldTop = useRef<number | null>(null);
  // Whether the browser put the list exactly at the held position.
  const heldLanded = useRef(false);
  // With unread messages the list opens at the first of them, which may
  // take older pages first. Until it settled it is not at the newest, so
  // nothing is marked read. `pending` waits for the read state.
  const opening = useRef<"pending" | "seek" | "done">("pending");
  const seekPages = useRef(0);
  const seekBudget = useRef(UNREAD_SEEK_PAGES);
  const lastJump = useRef(unreadJump);
  const edit = useEditMessage(channelId);
  const remove = useDeleteMessage(channelId);
  // Day dividers and the times beside the rows read the clock as they
  // render; this renders them again when the date changes.
  useLocalDay();
  // Rows that just came in live fade in (arrivals.ts); each row keeps what
  // it was told when it mounted.
  const [shownItems, setShownItems] = useState(items);
  const [arrived, setArrived] = useState<ReadonlyMap<string, Arrival>>(
    () => new Map(),
  );
  if (shownItems !== items) {
    setShownItems(items);
    setArrived(arrivals(shownItems, items, meId));
  }
  const revealed = useAfterSkeleton(!ready);
  const opensAt = unreadAnchor(items, newAfter ?? null, hasOlder);
  const newIndex = opensAt.kind === "row" ? opensAt.index : -1;

  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const message = items[index]!;
      const files = asAttachmentList(message.attachments);
      const images = files.filter((a) => isImageType(a.content_type)).length;
      const image = images > 0;
      // Players and file rows below the pictures.
      const extra = files.reduce(
        (sum, a) =>
          sum +
          (isVideoType(a.content_type)
            ? 280
            : isAudioType(a.content_type)
              ? 84
              : isImageType(a.content_type)
                ? 0
                : 44),
        0,
      );
      const breaks =
        (index === 0 && !hasOlder ? 120 : 0) +
        (dayBreak(items[index - 1], message) ? 40 : 0) +
        (index === newIndex ? 28 : 0) +
        extra +
        (message.reply ? 22 : 0);
      const gif = asGif(message.gif);
      if (gif)
        return (
          breaks +
          gifBox(gif).height +
          (isContinued(items[index - 1], message) ? 16 : 60)
        );
      // The image box is a fixed 240 px (chat.css), several pictures a grid
      // of 160 px rows, plus header and gaps.
      if (image) {
        const box = images > 1 ? Math.ceil(images / 2) * 164 : 240;
        return (
          breaks + box + (isContinued(items[index - 1], message) ? 16 : 60)
        );
      }
      return breaks + (isContinued(items[index - 1], message) ? 28 : 72);
    },
    getItemKey: (index) => items[index]?.id ?? index,
    overscan: 12,
  });

  useLayoutEffect(() => {
    itemsRef.current = items;
  }, [items]);

  // `pinEnd` is for size changes only. Scroll handlers just read: writing
  // scrollTop while the user scrolls cuts WebKit's momentum scrolling.
  const updateLatest = useCallback(
    (pinEnd = true) => {
      const element = scrollRef.current;
      if (!element) return;
      if (pinEnd && stickToBottom.current)
        element.scrollTop = element.scrollHeight;
      const position = {
        top: element.scrollTop,
        height: element.scrollHeight,
        viewport: element.clientHeight,
      };
      lastPosition.current = position;
      // The row at the top and its offset as of the last scroll or resize:
      // what a prepend or a vanished row must not move. From the
      // measurements, not the rendered range: this runs before the
      // virtualizer's own scroll listener, and after a long jump that range
      // still describes the old position.
      const first = virtualizer.getVirtualItemForOffset(element.scrollTop);
      const firstId = first ? itemsRef.current[first.index]?.id : undefined;
      if (first && firstId)
        pin.current = { id: firstId, offset: first.start - element.scrollTop };
      onAtLatest(
        active &&
          opening.current === "done" &&
          stickToBottom.current &&
          position.viewport > 16 &&
          endDistance(position) <= 16,
      );
    },
    [active, onAtLatest, virtualizer],
  );

  useLayoutEffect(() => {
    const element = scrollRef.current,
      content = contentRef.current;
    if (!element || !content) return;
    // The viewport also changes when the keyboard model, dock, or search
    // controls resize. Keep history intent even if the browser clamps to end.
    const observer = new ResizeObserver(() => updateLatest());
    observer.observe(element);
    observer.observe(content);
    updateLatest();
    return () => observer.disconnect();
  }, [updateLatest]);

  useLayoutEffect(() => {
    // Preserve history anchors; while pinned, the end owns resize adjustments.
    virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (
      item,
      _delta,
      instance,
    ) =>
      !stickToBottom.current &&
      // A held anchor sets the position itself, absolutely.
      !hold.current &&
      item.start < (instance.scrollOffset ?? 0);
  }, [virtualizer]);

  const totalSize = virtualizer.getTotalSize();
  // Fresh and unclamped: getTotalSize() recomputes the measurements from
  // the sizes measured so far. getOffsetForIndex() would clamp to a scroll
  // height the list has not grown to yet, and so jump (#201).
  const startOf = useCallback(
    (index: number) => {
      virtualizer.getTotalSize();
      return virtualizer.measurementsCache[index]?.start;
    },
    [virtualizer],
  );
  // The reader moved the list: from here on the position is theirs.
  const release = () => {
    hold.current = null;
    heldTop.current = null;
    opening.current = "done";
  };
  // Any difference from where the hold put the list, unless the browser
  // clamped that position: the rows above were not laid out yet, and the
  // browser moves a clamped scrollTop on its own as they are.
  const movedByReader = (top: number) =>
    heldTop.current !== null && heldLanded.current && top !== heldTop.current;
  useLayoutEffect(() => {
    if (latestRequest !== lastRequest.current) {
      stickToBottom.current = true;
      lastRequest.current = latestRequest;
      opening.current = "done";
    }
    if (unreadJump !== lastJump.current) {
      lastJump.current = unreadJump;
      if (newAfter) {
        opening.current = "seek";
        seekBudget.current = seekPages.current + UNREAD_SEEK_PAGES;
      }
    }
    if (opening.current === "pending" && ready && newAfter !== undefined)
      opening.current = newAfter ? "seek" : "done";
    if (opening.current === "seek" && loadError) opening.current = "done";
    // Where the first unread row goes, once it is loaded (or the pages to
    // look for it are used up: then the oldest loaded one).
    let unreadTop: { id: string; offset: number } | null = null;
    if (opening.current === "seek") {
      const anchor = unreadAnchor(items, newAfter ?? null, hasOlder);
      const settled =
        anchor.kind !== "older" || seekPages.current >= seekBudget.current;
      if (settled) {
        opening.current = "done";
        const index =
          anchor.kind === "row"
            ? anchor.index
            : anchor.kind === "older"
              ? 0
              : -1;
        const start = index >= 0 ? startOf(index) : undefined;
        const viewport = scrollRef.current?.clientHeight ?? 0;
        // Unread rows that fit below the top third stay at the end, where
        // they are all in view and get marked read.
        if (
          start !== undefined &&
          virtualizer.getTotalSize() - start > (viewport * 2) / 3
        ) {
          stickToBottom.current = false;
          // Row offsets count from the rows' box, below the paging line.
          const above = contentRef.current?.offsetTop ?? 0;
          unreadTop = {
            id: items[index]!.id,
            offset: Math.round(viewport / 3) - above,
          };
        }
        onOpened(unreadTop !== null);
      }
    }
    // The reader moved the list since the hold was applied, possibly before
    // the scroll event fired: even one pixel is theirs.
    if (scrollRef.current && movedByReader(scrollRef.current.scrollTop))
      release();
    const prepended = olderAnchor.current && items.length > lastCount.current;
    const firstChanged =
      lastFirst.current !== undefined && items[0]?.id !== lastFirst.current;

    if (prepended) {
      // Keep the row that was on top at the same place, offset included,
      // also while the new rows above it are measured in later commits.
      olderAnchor.current = false;
      hold.current = pin.current;
    } else if (stickToBottom.current) {
      // Native bottom pin below owns the end. scrollToIndex's asynchronous
      // measurement retries can otherwise pull a measured list back upwards.
      hold.current = null;
      heldTop.current = null;
    } else if (
      (items.length < lastCount.current || firstChanged) &&
      pin.current
    ) {
      // A row vanished (own delete or WS), or the oldest rows moved out of
      // the list. Keep the first visible id at the same offset so the list
      // does not jump.
      hold.current = pin.current;
    }
    if (unreadTop) hold.current = unreadTop;
    if (hold.current && scrollRef.current) {
      const top = anchoredScrollTop(items, hold.current, startOf);
      if (top === undefined) {
        // The row itself is gone: nothing left to hold.
        hold.current = null;
        heldTop.current = null;
      } else {
        scrollRef.current.scrollTop = top;
        heldTop.current = scrollRef.current.scrollTop;
        heldLanded.current = Math.abs(heldTop.current - top) < 1;
      }
    }

    updateLatest();
    lastCount.current = items.length;
    lastFirst.current = items[0]?.id;
    // `updateLatest()` pinned the row at scrollTop. Not the first rendered
    // row: that can be an overscan row far above the viewport.
  }, [
    items,
    virtualizer,
    startOf,
    updateLatest,
    totalSize,
    latestRequest,
    unreadJump,
    newAfter,
    ready,
    hasOlder,
    loadError,
    onOpened,
  ]);

  // Older pages until the last read message is loaded, also when
  // "Springen" adds pages to look through.
  useEffect(() => {
    if (
      opening.current !== "seek" ||
      !ready ||
      !hasOlder ||
      loadingOlder ||
      loadError ||
      seekPages.current >= seekBudget.current ||
      unreadAnchor(items, newAfter ?? null, hasOlder).kind !== "older"
    )
      return;
    seekPages.current += 1;
    onLoadOlder();
  }, [
    items,
    newAfter,
    hasOlder,
    loadingOlder,
    loadError,
    ready,
    onLoadOlder,
    unreadJump,
  ]);

  const [highlighted, setHighlighted] = useState<string | null>(null);
  const handledFocus = useRef(focusRequest?.n);
  useEffect(() => {
    if (!focusRequest || focusRequest.n === handledFocus.current) return;
    handledFocus.current = focusRequest.n;
    const index = itemsRef.current.findIndex(
      (message) => message.id === focusRequest.id,
    );
    if (index < 0) return;
    // A jump into history is the reader's position, not the end's.
    stickToBottom.current = false;
    hold.current = null;
    heldTop.current = null;
    opening.current = "done";
    virtualizer.scrollToIndex(index, { align: "center" });
    setHighlighted(focusRequest.id);
    const done = window.setTimeout(() => setHighlighted(null), 2000);
    return () => window.clearTimeout(done);
  }, [focusRequest, virtualizer]);

  const firstVisible = virtualizer.getVirtualItems()[0]?.index ?? 0;
  const firstId = items[0]?.id;
  useEffect(() => {
    if (!active || !ready || !hasOlder || loadingOlder || loadError) return;
    // First paint is pinned to the newest row; do not walk older pages
    // until the user actually scrolls up.
    if (stickToBottom.current) return;
    if (firstVisible > 4) return;
    olderAnchor.current = true;
    onLoadOlder();
  }, [
    active,
    firstVisible,
    firstId,
    hasOlder,
    loadingOlder,
    onLoadOlder,
    ready,
    loadError,
  ]);

  return (
    <div
      ref={scrollRef}
      role="log"
      aria-label="Nachrichten"
      aria-busy={!ready || undefined}
      onWheelCapture={(event) => {
        if (
          event.target instanceof Element &&
          event.target.closest("dialog:modal")
        )
          return;
        release();
        if (
          event.deltaY < 0 &&
          event.currentTarget.scrollHeight >
            event.currentTarget.clientHeight + 16
        )
          stickToBottom.current = false;
        else if (
          event.deltaY > 0 &&
          lastPosition.current &&
          endDistance(lastPosition.current) < 96
        )
          stickToBottom.current = true;
        updateLatest(false);
      }}
      onKeyDownCapture={(event) => {
        if (
          event.target instanceof HTMLElement &&
          event.target.closest(
            "dialog:modal,input,textarea,select,[contenteditable='true']",
          )
        )
          return;
        release();
        if (
          ["ArrowUp", "PageUp", "Home"].includes(event.key) &&
          event.currentTarget.scrollHeight >
            event.currentTarget.clientHeight + 16
        )
          stickToBottom.current = false;
        if (event.key === "End") stickToBottom.current = true;
        updateLatest(false);
      }}
      onTouchStartCapture={(event) => {
        if (
          event.target instanceof Element &&
          event.target.closest("dialog:modal")
        )
          return;
        touchY.current = event.touches[0]?.clientY ?? null;
      }}
      onTouchMoveCapture={(event) => {
        if (
          event.target instanceof Element &&
          event.target.closest("dialog:modal")
        )
          return;
        const current = event.touches[0]?.clientY;
        if (current === undefined || touchY.current === null) return;
        release();
        if (
          current > touchY.current &&
          event.currentTarget.scrollHeight >
            event.currentTarget.clientHeight + 16
        )
          stickToBottom.current = false;
        else if (
          current < touchY.current &&
          lastPosition.current &&
          endDistance(lastPosition.current) < 96
        )
          stickToBottom.current = true;
        touchY.current = current;
        updateLatest(false);
      }}
      onTouchEndCapture={() => {
        touchY.current = null;
      }}
      onPointerDownCapture={(event) => {
        // On the list itself, not a row: its scrollbar.
        if (event.target === event.currentTarget) release();
      }}
      onScrollCapture={(event) => {
        if (event.target !== event.currentTarget) return;
        const element = scrollRef.current;
        if (!element) return;
        // Momentum or a scrollbar drag moved it away from the held anchor.
        if (movedByReader(element.scrollTop)) release();
        // Observe intent before virtualizer measurement adjusts the geometry.
        stickToBottom.current = scrollEndIntent(
          stickToBottom.current,
          lastPosition.current,
          {
            top: element.scrollTop,
            height: element.scrollHeight,
            viewport: element.clientHeight,
          },
        );
        updateLatest(false);
      }}
      style={{ overflowAnchor: "none", overscrollBehavior: "contain" }}
      className="relative flex min-h-0 flex-1 flex-col overflow-y-auto"
    >
      {loadError ? (
        <div role="alert" className="lr-list-state">
          <p>
            {loadError === "paging"
              ? "Ältere Nachrichten konnten nicht geladen werden."
              : "Nachrichten konnten nicht geladen werden."}
          </p>
          <button
            type="button"
            onClick={onRetry}
            disabled={!ready || loadingOlder}
            className="lr-button-secondary"
          >
            Erneut laden
          </button>
        </div>
      ) : null}
      {items.length > 0 || hasOlder || loadingOlder ? (
        // Always there with one height: a row above the list that changes
        // size shifts everything below it.
        <p className="lr-list-loading">
          {loadingOlder ? "Ältere Nachrichten…" : ""}
        </p>
      ) : null}
      <div
        // Pin a short list to the bottom with flex, not a viewport-sized
        // margin: that ResizeObserver loop (scrollbar on/off) is React #185.
        ref={contentRef}
        style={{ height: virtualizer.getTotalSize() }}
        className={`relative mt-auto w-full shrink-0 ${revealed ? "ui-reveal" : ""}`}
      >
        {virtualizer.getVirtualItems().map((row) => {
          const message = items[row.index];
          if (!message) return null;
          const previous = items[row.index - 1];
          const newDay = dayBreak(previous, message);
          const firstNew = row.index === newIndex;
          const continued =
            !newDay && !firstNew && isContinued(previous, message);
          return (
            <div
              key={row.key}
              data-index={row.index}
              ref={virtualizer.measureElement}
              className={`absolute inset-x-0 ${message.id === highlighted ? "lr-message-highlight" : ""}`}
              style={{ top: row.start }}
            >
              {row.index === 0 && !hasOlder ? historyStart : null}
              {newDay ? (
                <div className="lr-day-divider" role="separator">
                  <span>{dayLabel(message.created_at)}</span>
                </div>
              ) : null}
              {firstNew ? (
                <div
                  className="lr-new-divider"
                  role="separator"
                  aria-label="Neue Nachrichten"
                >
                  <span>Neu</span>
                </div>
              ) : null}
              <MessageRow
                message={message}
                continued={continued}
                arrival={arrived.get(message.id)}
                mine={message.author.id === meId}
                canSend={canSend}
                canDelete={message.author.id === meId || canModerate}
                attempt={attempts.get(message.id)}
                onEdit={(content, failed) =>
                  edit.mutate({ id: message.id, content }, { onError: failed })
                }
                onDelete={async (skipConfirm) => {
                  if (skipConfirm || (await confirmDelete(message, meId)))
                    remove.mutate(message.id);
                }}
              />
            </div>
          );
        })}
      </div>
      {/* Below the empty rows box, which pins them to the bottom. */}
      {!ready && !loadError ? (
        <SkeletonRows count={6} avatars className="lr-list-skeleton" />
      ) : null}
      {ready && !loadError && items.length === 0 ? (
        <div className="lr-list-empty">
          {historyStart}
          <p>Noch keine Nachrichten. Schreib die erste.</p>
        </div>
      ) : null}
    </div>
  );
}

function confirmDelete(message: Message, meId: string | undefined) {
  const own = message.author.id === meId;
  return confirmAction({
    title: own
      ? "Nachricht löschen?"
      : `Nachricht von ${message.author.name} löschen?`,
    message: "Das lässt sich nicht rückgängig machen.",
    confirmLabel: "Löschen",
  });
}

/** Above the oldest message: the conversation has nothing older. */
function HistoryStart({
  mention,
  channelName,
  peer,
}: {
  mention: "#" | "@";
  channelName: string;
  peer?: { name: string; avatar_url: string | null };
}) {
  if (mention === "@")
    return (
      <div className="lr-history-start">
        {peer ? (
          <Avatar name={peer.name} url={peer.avatar_url} size="lg" decorative />
        ) : null}
        <h2>{peer?.name ?? channelName}</h2>
        <p>Das ist der Anfang eurer Unterhaltung.</p>
      </div>
    );
  return (
    <div className="lr-history-start">
      <h2>
        <HashIcon size={20} />
        {channelName}
      </h2>
      <p>Das ist der Anfang dieses Kanals.</p>
    </div>
  );
}
