// Text-channel body: virtualised history, optimistic composer, own
// edit/delete. A send writes the pending overlay first, so the row is
// painted in the same frame; the server answer swaps the `tmp:` id, an
// error retains an independently recoverable send attempt.

import { anchoredScrollTop } from "./scrollAnchor.ts";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  createContext,
  useContext,
  useCallback,
  useEffect,
  useEffectEvent,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  type Ref,
} from "react";

import {
  endDistance,
  scrollEndIntent,
  type ScrollPosition,
} from "../messages/scrollPosition.ts";
import { MessageContent } from "../messages/MessageContent.tsx";
import { MessageSearch } from "../messages/MessageSearch.tsx";
import { MessageContextView } from "../messages/MessageContext.tsx";
import { setReplyTarget, useReplyTarget } from "../messages/replyTarget.ts";
import { useChatDraft } from "../messages/drafts.ts";
import {
  readBoundary,
  useMarkManyRead,
  useMarkRead,
  useReadState,
} from "../messages/readState.ts";

import { useSession } from "../auth/session.ts";
import { fieldMessage } from "../auth/rules.ts";
import {
  arrivedSince,
  confirmedPendingIds,
  isContinued,
  UNREAD_SEEK_PAGES,
  unreadAnchor,
  visibleMessages,
} from "../messages/pages.ts";
import {
  nonePending,
  discardAttempt,
  displayPending,
  type SendAttempt,
  removePending,
  usePendingMessages,
} from "../messages/pending.ts";
import {
  isPendingId,
  useDeleteMessage,
  useEditMessage,
  useMessages,
  useSendMessage,
} from "../messages/queries.ts";
import {
  carriesFiles,
  droppedFiles,
  pastedFiles,
} from "../messages/clipboard.ts";
import {
  ACCEPT,
  CONTENT_MAX,
  formatSize,
  isAudioType,
  isImageType,
  isVideoType,
  validateAttachment,
  validateAttachmentSet,
  validateContent,
} from "../messages/rules.ts";
import type { Gif, Message, Reply } from "../messages/types.ts";
import {
  asAttachmentList,
  asGif,
  asReactionList,
  asReply,
} from "../messages/types.ts";
import { gifBox, type GifPick } from "../gifs/api.ts";
import { useGifStatus } from "../gifs/queries.ts";
import { useReducedMotion } from "../interface/useReducedMotion.ts";
import GifPicker, { GifMessageButton } from "./GifPicker.tsx";
import { Avatar } from "./Avatar.tsx";
import { sizeByDraft, type DraftHeight } from "./composerHeight.ts";
import { EmojiPickerLoader, ReactionBar } from "./ReactionBar.tsx";
import { useReaction } from "./useReaction.ts";
import { confirmAction } from "./confirm.ts";
import { Modal } from "./Modal.tsx";
import { ImageLightbox, type LightboxImage } from "./ImageLightbox.tsx";
import { LinkPreviews } from "./LinkPreviews.tsx";
import {
  ArrowDownIcon,
  CloseIcon,
  HashIcon,
  MoreIcon,
  PaperclipIcon,
  PencilIcon,
  ReplyIcon,
  SearchIcon,
  SendIcon,
  SmileIcon,
  WarningIcon,
} from "./Icons.tsx";
import {
  MessageActionSheet,
  type MessageActions,
} from "./MessageActionSheet.tsx";
import { longPressGesture, swallowNextClick } from "./longPress.ts";
import { Button } from "./ui/Button.tsx";
import { SkeletonRows } from "./ui/Skeleton.tsx";
import { copyText } from "../servers/clipboard.ts";
import { notify, notifyError } from "./toasts.ts";
import { AttachmentList } from "./Attachments.tsx";
import { OpenImage } from "./openImage.ts";
import { useMentionRenderer } from "./mentionRenderer.tsx";
import { MentionContext, useMentions } from "./mentionContext.ts";
import { useMentionSuggestions } from "./useMentionSuggestions.tsx";
import {
  fromWire,
  namesIn,
  mentionsMe,
  readableMentions,
  toWire,
  type MentionMember,
} from "../messages/mentions.ts";
import { NotificationBell } from "./NotificationSettingsDialog.tsx";
import type { NotificationTarget } from "./notificationTarget.ts";
import { useMuteClock } from "../messages/notificationSettings.ts";
import { isMuted } from "../messages/notifyLevel.ts";
import "./chat.css";

// Shows an answered message: in the list when it is loaded, else in its
// context from the server.
const JumpTo = createContext<(id: string) => void>(() => {});
const NO_MEMBERS: readonly MentionMember[] = [];

/** What a row that was not sent offers besides "Verwerfen". */
type AttemptActions = {
  retry: (attempt: SendAttempt) => void;
  /** Back into the composer, to change it before sending again. */
  edit: (attempt: SendAttempt) => void;
  canSendFiles: boolean;
};
const AttemptActionsContext = createContext<AttemptActions | null>(null);

type ComposerHandle = { takeBack: (attempt: SendAttempt) => void };

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
  const [newAfter, setNewAfter] = useState<string | null | undefined>(
    undefined,
  );
  // How many were unread then, for the bar; null once they are read.
  const [unreadCount, setUnreadCount] = useState<number | null>(null);
  if (newAfter === undefined && (readRows || readStateFailed)) {
    const unread = readRow && readRow.unread_count > 0 ? readRow : undefined;
    setNewAfter(unread ? unread.read_message_id : null);
    setUnreadCount(unread?.read_message_id ? unread.unread_count : null);
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

  // The newest row when the reader left the end: what others wrote since
  // is counted on the way back.
  const [awayFrom, setAwayFrom] = useState<string | null>(null);
  const newestId = items.at(-1)?.id;
  if (atLatest && awayFrom !== null) setAwayFrom(null);
  else if (!atLatest && awayFrom === null && newestId) setAwayFrom(newestId);
  const arrived = awayFrom ? arrivedSince(items, awayFrom, user?.id) : 0;
  const readIndex = newAfter
    ? items.findIndex((message) => message.id === newAfter)
    : -1;
  const firstUnread = readIndex >= 0 ? items[readIndex + 1] : undefined;

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
                          : `${unreadCount} neue Nachrichten`}
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
                      onClick={() => markReadNow()}
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
                          onEdit={(content) =>
                            contextEdit.mutate({ id: message.id, content })
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
                          onEdit={(content) =>
                            contextEdit.mutate({ id: message.id, content })
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
          onShowAttempt={jumpTo}
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
  newAfter: string | null | undefined;
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
  const readIndex = newAfter
    ? items.findIndex((message) => message.id === newAfter)
    : -1;
  const newIndex =
    readIndex >= 0 && readIndex + 1 < items.length ? readIndex + 1 : -1;

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

  // Older pages until the last read message is loaded.
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
  }, [items, newAfter, hasOlder, loadingOlder, loadError, ready, onLoadOlder]);

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
        className="relative mt-auto w-full shrink-0"
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
              className={`absolute inset-x-0${message.id === highlighted ? " lr-message-highlight" : ""}`}
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
                mine={message.author.id === meId}
                canSend={canSend}
                canDelete={message.author.id === meId || canModerate}
                attempt={attempts.get(message.id)}
                onEdit={(content) => edit.mutate({ id: message.id, content })}
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

function MessageRow({
  message,
  continued,
  mine,
  canSend,
  canDelete,
  attempt,
  onEdit,
  onDelete,
}: {
  message: Message;
  continued: boolean;
  mine: boolean;
  canSend: boolean;
  canDelete: boolean;
  /** The send attempt of a `tmp:` row. */
  attempt?: SendAttempt;
  onEdit: (content: string) => void;
  /** `skipConfirm` with Shift held. */
  onDelete: (skipConfirm: boolean) => void;
}) {
  const pending = isPendingId(message.id);
  const { members, me } = useMentions();
  const mentioned = !mine && mentionsMe(message, me);
  const renderMention = useMentionRenderer(message);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const editInput = useRef<HTMLTextAreaElement>(null);
  const editSuggestions = useMentionSuggestions(draft, (text, caret) => {
    setDraft(text);
    requestAnimationFrame(() =>
      editInput.current?.setSelectionRange(caret, caret),
    );
  });
  const sentNames = useMemo(
    () => namesIn(message.content, members),
    [message.content, members],
  );
  const wireDraft = toWire(
    draft,
    members,
    new Map([...sentNames, ...editSuggestions.picked]),
  );
  const rowRef = useRef<HTMLDivElement>(null);
  const editOpener = useRef<HTMLElement | null>(null);
  const restoreEditFocus = useRef(false);
  useLayoutEffect(() => {
    if (!editing && restoreEditFocus.current) {
      restoreEditFocus.current = false;
      const opener = editOpener.current;
      (opener?.isConnected ? opener : rowRef.current)?.focus({
        preventScroll: true,
      });
    }
  }, [editing]);
  const finishEditing = () => {
    restoreEditFocus.current = true;
    setEditing(false);
  };

  const startEdit = () => {
    restoreEditFocus.current = false;
    // Back to the edit button afterwards, or to the row when the edit came
    // from the action sheet.
    const active = document.activeElement;
    editOpener.current =
      active instanceof HTMLElement && rowRef.current?.contains(active)
        ? active
        : rowRef.current;
    setDraft(fromWire(message.content, members));
    setEditing(true);
  };
  const cancelEdit = () => {
    finishEditing();
    setDraft(fromWire(message.content, members));
  };

  const moreButton = useRef<HTMLButtonElement>(null);
  // The actions menu, at the "Mehr" button or for the whole row (sheet).
  const [menu, setMenu] = useState<"more" | "row" | null>(null);
  const [gesture] = useState(() =>
    longPressGesture(() => {
      navigator.vibrate?.(10);
      swallowNextClick();
      setMenu("row");
    }),
  );
  const reaction = useReaction(message, canSend);

  const when = formatWhen(message.created_at);
  const fullWhen = formatFull(message.created_at);
  const error = editing
    ? draft.length === 0
      ? message.attachments.length > 0
        ? null
        : validateContent(wireDraft)
      : validateContent(wireDraft)
    : null;
  // A `tmp:` row is being sent until its attempt says otherwise.
  const sendState = pending ? (attempt?.status ?? "sending") : null;
  const showDelete = canDelete && !pending && !editing;
  const gif = asGif(message.gif);
  const showEdit = mine && canSend && !pending && !editing && !gif;
  const canReact = canSend && !pending && !editing;
  const canReply = canSend && !pending && !editing;
  const reply = asReply(message.reply);
  const [reactionSlot, setReactionSlot] = useState<HTMLSpanElement | null>(
    null,
  );

  const copy = async () => {
    if (await copyText(fromWire(message.content, members)))
      notify("Text kopiert.");
    else notifyError(new Error("Der Text konnte nicht kopiert werden."));
  };
  const menuActions: MessageActions =
    pending || editing
      ? {}
      : {
          react: canReact ? reaction.toggle : undefined,
          moreReactions: canReact ? () => reaction.setPicking(true) : undefined,
          reply: canReply
            ? () => setReplyTarget(message.channel_id, message)
            : undefined,
          edit: showEdit ? startEdit : undefined,
          copy: message.content ? () => void copy() : undefined,
          remove: showDelete ? onDelete : undefined,
        };
  const hasMenu =
    !pending &&
    !editing &&
    (canReact ||
      canReply ||
      showEdit ||
      showDelete ||
      Boolean(message.content));
  const myReactions = asReactionList(message.reactions)
    .filter((entry) => entry.user_ids.includes(reaction.userId))
    .map((entry) => entry.emoji);

  // A finger held on the row opens the sheet; a right click the menu,
  // except on a link, a picture or selected text, which keep the browser's.
  const menuHandlers = hasMenu
    ? {
        onPointerDown: (event: PointerEvent<HTMLDivElement>) => {
          if ((event.target as Element).closest("textarea, input")) return;
          gesture.down({
            pointerType: event.pointerType,
            isPrimary: event.isPrimary,
            x: event.clientX,
            y: event.clientY,
          });
        },
        onPointerMove: (event: PointerEvent<HTMLDivElement>) =>
          gesture.move({ x: event.clientX, y: event.clientY }),
        onPointerUp: gesture.end,
        onPointerCancel: gesture.end,
        onPointerLeave: gesture.end,
        onContextMenu: (event: MouseEvent<HTMLDivElement>) => {
          const pointer = (
            event.nativeEvent as Partial<globalThis.PointerEvent>
          ).pointerType;
          // Android's own long press: the gesture above answers it.
          if (pointer === "touch" || pointer === "pen") {
            event.preventDefault();
            return;
          }
          if (
            (event.target as Element).closest(
              "a, img, video, audio, input, textarea",
            ) ||
            String(window.getSelection() ?? "").trim()
          )
            return;
          event.preventDefault();
          setMenu("more");
        },
      }
    : {};

  const actions = hasMenu ? (
    <span className="lr-message-actions">
      <span ref={setReactionSlot} className="lr-message-reaction-slot" />
      {canReply ? (
        <IconButton
          label="Antworten"
          onClick={() => setReplyTarget(message.channel_id, message)}
        >
          <ReplyIcon size={14} />
        </IconButton>
      ) : null}
      {showEdit ? (
        <IconButton label="Nachricht bearbeiten" onClick={startEdit}>
          <PencilIcon size={14} />
        </IconButton>
      ) : null}
      <IconButton
        buttonRef={moreButton}
        label="Weitere Aktionen"
        menu={menu === "more"}
        onClick={() => setMenu(menu === "more" ? null : "more")}
      >
        <MoreIcon size={14} />
      </IconButton>
    </span>
  ) : null;
  const sheet = hasMenu ? (
    <MessageActionSheet
      open={menu !== null}
      onClose={() => setMenu(null)}
      anchor={menu === "row" ? rowRef : moreButton}
      author={message.author.name}
      actions={menuActions}
      mine={myReactions}
    />
  ) : null;
  const status = sendState ? <SendStatus state={sendState} /> : null;
  const failure =
    attempt && attempt.status !== "sending" ? (
      <SendFailure attempt={attempt} />
    ) : null;
  const edited = message.edited_at ? (
    <span
      className="lr-message-edited"
      title={`Bearbeitet am ${formatFull(message.edited_at)}`}
    >
      (bearbeitet)
    </span>
  ) : null;
  const rowClass = [
    "lr-message-row group flex items-start gap-3",
    continued ? "lr-message-continued" : "",
    sendState === "sending" ? "is-sending" : "",
    sendState === "failed" || sendState === "uncertain" ? "is-failed" : "",
    mentioned ? "is-mentioned" : "",
  ].join(" ");

  if (continued && !editing) {
    return (
      <div ref={rowRef} tabIndex={-1} className={rowClass} {...menuHandlers}>
        <span className="lr-message-gutter" aria-hidden="true">
          {sendState ? null : (
            <time dateTime={message.created_at} title={fullWhen}>
              {formatClock(message.created_at)}
            </time>
          )}
        </span>
        <div className="min-w-0 flex-1">
          <span className="sr-only">
            {message.author.name}, {when}
          </span>
          {message.content ? (
            <div className="lr-message-text">
              <MessageContent
                content={message.content}
                renderMention={renderMention}
              />
              {edited}
            </div>
          ) : null}
          {gif ? <GifMessage gif={gif} /> : null}
          <AttachmentList attachments={message.attachments ?? []} />
          <LinkPreviews
            messageId={message.id}
            previews={message.previews}
            canRemove={canDelete && !pending}
          />
          <ReactionBar
            message={message}
            canSend={canSend}
            reaction={reaction}
            addSlot={reactionSlot}
          />
          {status ? <p className="lr-message-status-line">{status}</p> : null}
          {failure}
        </div>
        {actions}
        {sheet}
      </div>
    );
  }

  return (
    <div ref={rowRef} tabIndex={-1} className={rowClass} {...menuHandlers}>
      <Avatar name={message.author.name} url={message.author.avatar_url} />
      <div className="min-w-0 flex-1">
        {reply ? <ReplyHeader reply={reply} /> : null}
        <div className="flex flex-wrap items-baseline gap-2">
          <span className="lr-message-author">{message.author.name}</span>
          {status ?? (
            <time
              dateTime={message.created_at}
              title={fullWhen}
              className="lr-message-time"
            >
              {when}
            </time>
          )}
        </div>
        {editing ? (
          <form
            className="lr-message-edit"
            onSubmit={(event) => {
              event.preventDefault();
              if (error) return;
              onEdit(wireDraft);
              finishEditing();
            }}
          >
            {editSuggestions.list}
            <textarea
              ref={editInput}
              value={draft}
              {...editSuggestions.fieldProps}
              onChange={(e) => {
                setDraft(e.target.value);
                editSuggestions.track(e.target);
              }}
              onKeyDown={(event) => {
                if (editSuggestions.onKeyDown(event)) return;
                if (event.key === "Escape") {
                  event.preventDefault();
                  cancelEdit();
                }
                if (
                  event.key === "Enter" &&
                  !event.shiftKey &&
                  !event.nativeEvent.isComposing
                ) {
                  event.preventDefault();
                  if (!error) {
                    onEdit(wireDraft);
                    finishEditing();
                  }
                }
              }}
              rows={2}
              aria-label="Nachricht bearbeiten"
              autoFocus
            />
            {error ? (
              <p className="lr-field-error">{fieldMessage("content", error)}</p>
            ) : null}
            <div className="lr-message-edit-actions">
              <span>Esc bricht ab · Enter speichert</span>
              <button type="button" onClick={cancelEdit}>
                Abbrechen
              </button>
              <button type="submit" disabled={Boolean(error)}>
                Speichern
              </button>
            </div>
          </form>
        ) : message.content || !gif ? (
          <div className="lr-message-text">
            <MessageContent
              content={message.content}
              renderMention={renderMention}
            />
            {edited}
          </div>
        ) : null}
        {!editing && gif ? <GifMessage gif={gif} /> : null}
        {!editing ? (
          <AttachmentList attachments={message.attachments ?? []} />
        ) : null}
        {!editing ? (
          <LinkPreviews
            messageId={message.id}
            previews={message.previews}
            canRemove={canDelete && !pending}
          />
        ) : null}
        {!editing && (
          <ReactionBar
            message={message}
            canSend={canSend}
            reaction={reaction}
            addSlot={reactionSlot}
          />
        )}
        {failure}
      </div>
      {actions}
      {sheet}
    </div>
  );
}

/** In place of the time while a message is on its way or did not arrive. */
function SendStatus({ state }: { state: SendAttempt["status"] }) {
  if (state === "sending")
    return (
      <span className="lr-message-status is-sending">
        <span className="lr-message-spinner" aria-hidden="true" />
        Wird gesendet…
      </span>
    );
  return (
    <span className="lr-message-status is-failed">
      <WarningIcon size={13} />
      {state === "failed" ? "Nicht gesendet" : "Status unklar – Verlauf prüfen"}
    </span>
  );
}

/** Why a message was not sent, and what to do about it, on its row. */
function SendFailure({ attempt }: { attempt: SendAttempt }) {
  const actions = useContext(AttemptActionsContext);
  return (
    <div className="lr-message-failure">
      <p>
        {attempt.error}
        {attempt.status === "uncertain"
          ? " Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf, bevor du sie erneut sendest."
          : null}
      </p>
      <div className="lr-message-failure-actions">
        {attempt.status === "failed" && actions ? (
          <Button
            size="sm"
            variant="secondary"
            disabled={attempt.files.length > 0 && !actions.canSendFiles}
            onClick={() => actions.retry(attempt)}
          >
            Erneut senden
          </Button>
        ) : null}
        {attempt.gif || !actions ? null : (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => actions.edit(attempt)}
          >
            Bearbeiten
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          onClick={() => discardAttempt(attempt.id)}
        >
          Verwerfen
        </Button>
      </div>
    </div>
  );
}

function Composer({
  handle,
  channelId,
  channelName,
  canSend,
  canSendFiles,
  mention,
  author,
  attempts,
  onSent,
  onShowAttempt,
  onDraftChange,
  onDraftStop,
}: {
  handle: Ref<ComposerHandle>;
  channelId: string;
  channelName: string;
  canSend: boolean;
  canSendFiles: boolean;
  mention: "#" | "@";
  author: { id: string; name: string; avatar_url: string | null } | null;
  attempts: SendAttempt[];
  /** A message left: the list shows the end, where it appears. */
  onSent: () => void;
  /** Scrolls to the row of a send that did not go through. */
  onShowAttempt: (id: string) => void;
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const send = useSendMessage(
    channelId,
    author ?? { id: "", name: "", avatar_url: null },
  );
  const [savedDraft, updateDraft, previews] = useChatDraft(
    author?.id ?? "",
    channelId,
  );
  const draft = savedDraft.text;
  const files = savedDraft.files;
  const setDraft = (text: string) => updateDraft({ text });
  const setFiles = (files: File[]) => updateDraft({ files });
  const [fileError, setFileError] = useState<string | null>(null);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [gifOpen, setGifOpen] = useState(false);
  const gifs = useGifStatus();
  const gifsEnabled = gifs.data?.enabled === true;
  const replyTarget = useReplyTarget(channelId);
  const fileInput = useRef<HTMLInputElement>(null);
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const composerForm = useRef<HTMLFormElement>(null);
  const keepKeyboard = useRef(false);
  const revealComposerFocus = useCallback(() => {
    const form = composerForm.current;
    const pane = form?.closest<HTMLElement>(".lr-message-pane");
    const focused = document.activeElement;
    if (
      !form ||
      !pane ||
      !(focused instanceof HTMLElement) ||
      !form.contains(focused)
    )
      return;
    const boundary = pane.getBoundingClientRect();
    const target = focused.getBoundingClientRect();
    const focusMargin = 4;
    if (target.bottom + focusMargin > boundary.bottom)
      pane.scrollTop += target.bottom + focusMargin - boundary.bottom;
    else if (target.top - focusMargin < boundary.top)
      pane.scrollTop -= boundary.top - target.top + focusMargin;
  }, []);
  // "Antworten" moves the keyboard to the field it fills.
  useEffect(() => {
    if (replyTarget) composerInput.current?.focus({ preventScroll: true });
  }, [replyTarget]);
  // Toolbar/read-state changes can move a focused form without resizing it.
  useLayoutEffect(revealComposerFocus, [revealComposerFocus]);
  useLayoutEffect(() => {
    const form = composerForm.current;
    const pane = form?.closest<HTMLElement>(".lr-message-pane");
    if (!form || !pane) return;
    const observer = new ResizeObserver(revealComposerFocus);
    observer.observe(form);
    observer.observe(pane);
    revealComposerFocus();
    return () => observer.disconnect();
  }, [canSend, revealComposerFocus]);
  // chat.css sizes the field by its draft on phones. Browsers without
  // field-sizing (iOS before 26.2) get the same height from composerHeight.
  const draftHeight = useRef<DraftHeight | null>(null);
  useLayoutEffect(() => {
    const input = composerInput.current;
    const form = composerForm.current;
    if (!input || !form) return;
    const sizing = sizeByDraft(window, input, form);
    draftHeight.current = sizing;
    return () => {
      draftHeight.current = null;
      sizing?.stop();
    };
  }, [canSend]);
  useLayoutEffect(() => draftHeight.current?.fit(), [draft, canSend]);
  const { members } = useMentions();
  const suggestions = useMentionSuggestions(draft, (text, caret) => {
    setDraft(text);
    onDraftChange?.(text);
    requestAnimationFrame(() =>
      composerInput.current?.setSelectionRange(caret, caret),
    );
  });
  // The field shows `@Name`; what is sent and counted is `<@id>`.
  const wire = toWire(draft, members, suggestions.picked);
  const error = draft.length === 0 ? null : validateContent(wire);
  const remaining = CONTENT_MAX - Array.from(normalisedLength(wire)).length;
  const emptyText = draft.trim().length === 0;
  const disabled =
    !canSend ||
    !author ||
    Boolean(error) ||
    Boolean(files.length > 0 && !canSendFiles) ||
    (emptyText && files.length === 0);

  /** Add files to `base` (the composer's files): invalid ones are left
   *  out, and so is whatever would go over the message's limits. One note
   *  names every file left out. */
  const addFiles = (next: File[], base: File[] = files) => {
    setFileError(null);
    let kept = [...base];
    const notes: string[] = [];
    const over = new Map<string, string[]>();
    for (const candidate of next) {
      const invalid = validateAttachment(candidate);
      if (invalid) {
        notes.push(
          `${candidate.name}: ${fieldMessage(invalid.field, invalid.code)}`,
        );
        continue;
      }
      // A smaller file further on may still fit under the size limit.
      const limit = validateAttachmentSet([...kept, candidate]);
      if (limit) {
        const reason = fieldMessage(limit.field, limit.code);
        over.set(reason, [...(over.get(reason) ?? []), candidate.name]);
        continue;
      }
      kept = [...kept, candidate];
    }
    for (const [reason, names] of over)
      notes.push(`Nicht angehängt: ${names.join(", ")}. ${reason}`);
    if (notes.length > 0) setFileError(notes.join(" "));
    setFiles(kept);
  };
  const removeFile = (index: number) => {
    setFileError(null);
    setFiles(files.filter((_, i) => i !== index));
  };

  // Files dropped anywhere on the conversation attach like the paperclip.
  const [dropping, setDropping] = useState(false);
  const attachDropped = useEffectEvent((next: File[]) => {
    addFiles(next);
    composerInput.current?.focus();
  });
  useEffect(() => {
    const pane = composerForm.current?.closest<HTMLElement>(".lr-message-pane");
    if (!pane || !canSendFiles) return;
    // enter/leave fire for every child the pointer crosses.
    let depth = 0;
    const show = (on: boolean) => {
      pane.classList.toggle("is-drop-target", on);
      setDropping(on);
    };
    const enter = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer)) return;
      event.preventDefault();
      depth += 1;
      show(true);
    };
    const over = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer) || !event.dataTransfer) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    };
    const leave = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) show(false);
    };
    const drop = (event: DragEvent) => {
      if (!carriesFiles(event.dataTransfer)) return;
      event.preventDefault();
      depth = 0;
      show(false);
      const dropped = droppedFiles(event.dataTransfer);
      if (dropped.length > 0) attachDropped(dropped);
    };
    pane.addEventListener("dragenter", enter);
    pane.addEventListener("dragover", over);
    pane.addEventListener("dragleave", leave);
    pane.addEventListener("drop", drop);
    return () => {
      pane.removeEventListener("dragenter", enter);
      pane.removeEventListener("dragover", over);
      pane.removeEventListener("dragleave", leave);
      pane.removeEventListener("drop", drop);
      show(false);
    };
  }, [canSend, canSendFiles]);

  const submit = (event?: FormEvent) => {
    event?.preventDefault();
    if (disabled) return;
    const text = wire;
    send.mutate({ content: text, files, replyTo: replyTarget });
    onSent();
    setReplyTarget(channelId, null);
    updateDraft({ text: "", files: [] });
    setFileError(null);
    onDraftStop?.();
    if (keepKeyboard.current) composerInput.current?.focus();
    keepKeyboard.current = false;
  };

  const sendGif = (pick: GifPick) => {
    setGifOpen(false);
    if (!canSend || !author) return;
    send.mutate({ content: "", gif: pick, replyTo: replyTarget });
    onSent();
    setReplyTarget(channelId, null);
    composerInput.current?.focus();
  };
  // "Bearbeiten" on a row that was not sent: its text and files come back
  // here, replacing what the composer holds.
  useImperativeHandle(handle, () => ({
    takeBack: (attempt) => {
      setDraft(fromWire(attempt.content, members));
      addFiles(attempt.files, []);
      discardAttempt(attempt.id);
      composerInput.current?.focus();
    },
  }));
  const unsent = attempts.filter((attempt) => attempt.status !== "sending");

  const insertText = (text: string) => {
    const input = composerInput.current;
    const start = input?.selectionStart ?? draft.length;
    const end = input?.selectionEnd ?? draft.length;
    const next = draft.slice(0, start) + text + draft.slice(end);
    setDraft(next);
    onDraftChange?.(next);
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(start + text.length, start + text.length);
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (suggestions.onKeyDown(event)) return;
    if (event.key === "Escape" && replyTarget) {
      event.preventDefault();
      setReplyTarget(channelId, null);
      return;
    }
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      submit();
    }
  };

  if (!canSend) {
    return (
      <p className="lr-composer-readonly">
        Du kannst in diesem Kanal nicht schreiben.
      </p>
    );
  }

  return (
    <form
      ref={composerForm}
      onFocusCapture={revealComposerFocus}
      onSubmit={submit}
      className="lr-composer"
    >
      {unsent.length > 0 ? (
        <div role="alert" className="lr-send-summary">
          <WarningIcon size={14} />
          <span>
            {unsent.length === 1
              ? "1 Nachricht nicht gesendet"
              : `${unsent.length} Nachrichten nicht gesendet`}
          </span>
          <button type="button" onClick={() => onShowAttempt(unsent[0]!.id)}>
            Anzeigen
          </button>
        </div>
      ) : null}
      <label className="sr-only" htmlFor={`compose-${channelId}`}>
        Nachricht in {mention}
        {channelName}
      </label>
      <div className="lr-composer-field">
        {replyTarget ? (
          <div className="lr-composer-reply">
            <ReplyIcon size={14} />
            <p>
              Antwort an <strong>{replyTarget.author.name}</strong>
              <span className="lr-composer-reply-hint"> · Esc bricht ab</span>
            </p>
            <button
              type="button"
              onClick={() => {
                setReplyTarget(channelId, null);
                composerInput.current?.focus();
              }}
              aria-label="Antworten abbrechen"
              title="Antworten abbrechen"
            >
              <CloseIcon size={14} />
            </button>
          </div>
        ) : null}
        {dropping ? (
          <p className="lr-composer-drop" aria-hidden="true">
            Loslassen zum Anhängen
          </p>
        ) : null}
        {files.length > 0 ? (
          <ul className="lr-composer-attachments" aria-label="Anhänge">
            {files.map((attached, index) => {
              const preview = previews.get(attached);
              return (
                <li
                  key={`${index}:${attached.name}:${attached.size}:${attached.lastModified}`}
                  className="lr-composer-attachment"
                >
                  {preview ? <img src={preview} alt="" /> : null}
                  <div>
                    <p>{attached.name}</p>
                    <p>{formatSize(attached.size)}</p>
                  </div>
                  <button
                    type="button"
                    onClick={() => removeFile(index)}
                    aria-label={`${attached.name} entfernen`}
                    title="Anhang entfernen"
                  >
                    <CloseIcon size={14} />
                  </button>
                </li>
              );
            })}
          </ul>
        ) : null}
        {suggestions.list}
        <div className="lr-composer-row">
          {canSendFiles ? (
            <>
              <input
                ref={fileInput}
                type="file"
                accept={ACCEPT}
                multiple
                className="sr-only"
                onChange={(event) => {
                  const selected = Array.from(event.target.files ?? []);
                  addFiles(selected);
                  event.target.value = "";
                  if (selected.length > 0) composerInput.current?.focus();
                }}
              />
              <button
                type="button"
                title="Datei anhängen"
                aria-label="Datei anhängen"
                onClick={() => fileInput.current?.click()}
                className="lr-composer-icon"
              >
                <PaperclipIcon size={18} />
              </button>
            </>
          ) : null}
          <textarea
            ref={composerInput}
            id={`compose-${channelId}`}
            value={draft}
            {...suggestions.fieldProps}
            onChange={(e) => {
              setDraft(e.target.value);
              onDraftChange?.(e.target.value);
              suggestions.track(e.target);
            }}
            onBlur={() => {
              suggestions.fieldProps.onBlur();
              onDraftStop?.();
            }}
            onKeyDown={onKeyDown}
            onPaste={(event) => {
              if (!canSendFiles) return;
              const pasted = pastedFiles(event.clipboardData);
              if (pasted.length === 0) return;
              event.preventDefault();
              addFiles(pasted);
            }}
            rows={1}
            placeholder={`Nachricht an ${mention}${channelName}`}
          />
          <button
            type="button"
            title="Emoji einfügen"
            aria-label="Emoji einfügen"
            onClick={() => setEmojiOpen(true)}
            className="lr-composer-icon"
          >
            <SmileIcon size={18} />
          </button>
          {gifsEnabled ? (
            <button
              type="button"
              title="GIF senden"
              aria-label="GIF senden"
              onClick={() => setGifOpen(true)}
              className="lr-composer-icon lr-composer-gif"
            >
              <span aria-hidden="true">GIF</span>
            </button>
          ) : null}
          <button
            type="submit"
            disabled={disabled}
            aria-label="Senden"
            title="Senden"
            // A tap would move focus to the button and close the on-screen
            // keyboard after every message.
            onPointerDown={(event) => {
              keepKeyboard.current =
                event.pointerType !== "mouse" &&
                document.activeElement === composerInput.current;
              if (keepKeyboard.current) event.preventDefault();
            }}
            className="lr-composer-send"
          >
            <SendIcon size={17} />
          </button>
        </div>
      </div>
      <Modal
        open={emojiOpen}
        onClose={() => setEmojiOpen(false)}
        title="Emoji einfügen"
        wide
      >
        {emojiOpen ? (
          <EmojiPickerLoader
            onSelect={(emoji) => {
              setEmojiOpen(false);
              insertText(emoji);
            }}
          />
        ) : null}
      </Modal>
      {gifsEnabled ? (
        <Modal
          open={gifOpen}
          onClose={() => setGifOpen(false)}
          title="GIF senden"
          wide
        >
          {gifOpen ? <GifPicker onSelect={sendGif} /> : null}
        </Modal>
      ) : null}
      <div className="lr-composer-status" role="status">
        {remaining < 200 ? (
          <span
            className={remaining < 0 ? "lr-field-error" : undefined}
            // Read out only near the limit, not with every letter.
            aria-live={remaining < 20 ? "polite" : "off"}
          >
            {remaining < 0
              ? `${-remaining} Zeichen zu viel`
              : `${remaining} Zeichen übrig`}
          </span>
        ) : null}
        {error ? (
          <span className="lr-field-error">
            {fieldMessage("content", error)}
          </span>
        ) : null}
      </div>
      {fileError ? (
        <p role="alert" className="lr-composer-file-error lr-field-error">
          {fileError}
        </p>
      ) : null}
    </form>
  );
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

function ReplyHeader({ reply }: { reply: Reply }) {
  const jumpTo = useContext(JumpTo);
  const { members } = useMentions();
  if (reply.deleted || !reply.author)
    return (
      <p className="lr-reply-header is-deleted">
        <ReplyIcon size={13} />
        <span>Die ursprüngliche Nachricht wurde gelöscht.</span>
      </p>
    );
  const summary =
    (reply.excerpt && readableMentions(reply.excerpt, members)) ||
    (reply.has_gif ? "GIF" : reply.has_attachment ? "Anhang" : "");
  return (
    <button
      type="button"
      className="lr-reply-header"
      onClick={() => jumpTo(reply.id)}
      aria-label={`Antwort auf ${reply.author.name}: ${summary}. Zur Nachricht springen`}
    >
      <ReplyIcon size={13} />
      <span className="lr-reply-author">{reply.author.name}</span>
      <span className="lr-reply-excerpt">
        {summary}
        {reply.excerpt && (reply.has_attachment || reply.has_gif) ? (
          <PaperclipIcon size={12} />
        ) : null}
      </span>
    </button>
  );
}

function GifMessage({ gif }: { gif: Gif }) {
  return <GifMessageButton gif={gif} reduced={useReducedMotion()} />;
}

function dayKey(iso: string): string {
  const date = new Date(iso);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function dayBreak(previous: Message | undefined, message: Message): boolean {
  return (
    !previous || dayKey(previous.created_at) !== dayKey(message.created_at)
  );
}

function dayLabel(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (dayKey(iso) === dayKey(today.toISOString())) return "Heute";
  if (dayKey(iso) === dayKey(yesterday.toISOString())) return "Gestern";
  return new Intl.DateTimeFormat("de-DE", {
    weekday: "long",
    day: "numeric",
    month: "long",
    ...(date.getFullYear() === today.getFullYear() ? {} : { year: "numeric" }),
  }).format(date);
}

function normalisedLength(raw: string): string {
  return raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
}

function formatWhen(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return new Intl.DateTimeFormat("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    ...(sameDay ? {} : { day: "2-digit", month: "2-digit" }),
  }).format(date);
}

/** "Dienstag, 21. Oktober 2026 um 14:02", for a time's tooltip. */
function formatFull(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat("de-DE", {
    dateStyle: "full",
    timeStyle: "short",
  }).format(date);
}

/** "14:02", beside a grouped message. */
function formatClock(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function IconButton({
  buttonRef,
  label,
  onClick,
  children,
  menu,
}: {
  buttonRef?: Ref<HTMLButtonElement>;
  label: string;
  onClick: () => void;
  children: ReactNode;
  /** Opens a menu: whether it is open. */
  menu?: boolean;
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      title={label}
      aria-label={label}
      aria-haspopup={menu === undefined ? undefined : "menu"}
      aria-expanded={menu}
      onClick={onClick}
      className="lr-message-action"
    >
      {children}
    </button>
  );
}
