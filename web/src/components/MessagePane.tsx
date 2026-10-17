// Text-channel body: virtualised history, optimistic composer, own
// edit/delete. A send writes the pending overlay first, so the row is
// painted in the same frame; the server answer swaps the `tmp:` id, an
// error retains an independently recoverable send attempt.

import { anchoredScrollTop } from "./scrollAnchor.ts";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useEffectEvent,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from "react";

import {
  endDistance,
  scrollEndIntent,
  type ScrollPosition,
} from "../messages/scrollPosition.ts";
import { MessageSearch } from "../messages/MessageSearch.tsx";
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
  confirmedPendingIds,
  isContinued,
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
import { attachmentDownloadUrl, attachmentUrl } from "../messages/api.ts";
import {
  carriesFiles,
  droppedFile,
  pastedFile,
} from "../messages/clipboard.ts";
import {
  ALLOWED_TYPES,
  CONTENT_MAX,
  isImageType,
  validateAttachment,
  validateContent,
} from "../messages/rules.ts";
import type { Attachment, Gif, Message } from "../messages/types.ts";
import { asAttachmentList, asGif } from "../messages/types.ts";
import { gifBox, gifLabel, type GifPick } from "../gifs/api.ts";
import { useGifStatus } from "../gifs/queries.ts";
import { useReducedMotion } from "../interface/useReducedMotion.ts";
import GifPicker, { GifMessageButton } from "./GifPicker.tsx";
import { Avatar } from "./Avatar.tsx";
import { sizeByDraft, type DraftHeight } from "./composerHeight.ts";
import { EmojiPickerLoader, ReactionBar } from "./ReactionBar.tsx";
import { confirmAction } from "./confirm.ts";
import { Modal } from "./Modal.tsx";
import { ImageLightbox, type LightboxImage } from "./ImageLightbox.tsx";
import { isPlainClick } from "./plainClick.ts";
import {
  ArrowDownIcon,
  CloseIcon,
  PaperclipIcon,
  PencilIcon,
  SearchIcon,
  SendIcon,
  SmileIcon,
  TrashIcon,
} from "./Icons.tsx";
import { MessageText } from "./MessageMentions.tsx";
import { MentionContext, useMentions } from "./mentionContext.ts";
import { useMentionSuggestions } from "./useMentionSuggestions.tsx";
import {
  fromWire,
  namesIn,
  mentionsMe,
  toWire,
  type MentionMember,
} from "../messages/mentions.ts";
import { NotificationBell } from "./NotificationSettingsDialog.tsx";
import type { NotificationTarget } from "./notificationTarget.ts";
import { useMuteClock } from "../messages/notificationSettings.ts";
import { isMuted } from "../messages/notifyLevel.ts";
import "./chat.css";

// Opens the one lightbox of the pane. It lives outside the virtual rows: a
// row scrolled out of the overscan unmounts, and would close it with it.
const OpenImage = createContext<(image: LightboxImage) => void>(() => {});
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
  const markByKey = useEffectEvent((server: boolean) => {
    const serverId = notify?.serverId;
    void markManyRead(
      server && serverId ? { server: serverId } : { channel: channelId },
    ).catch(() => {});
  });
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
  const { data: readRows } = useReadState();
  const [newAfter, setNewAfter] = useState<string | null | undefined>(
    undefined,
  );
  if (newAfter === undefined && readRows) {
    const row = readRows.find((entry) => entry.channel_id === channelId);
    setNewAfter(row && row.unread_count > 0 ? row.read_message_id : null);
  }
  const contextEdit = useEditMessage(channelId);
  const contextRemove = useDeleteMessage(channelId);
  const [atLatest, setAtLatest] = useState(false);
  const [latestRequest, setLatestRequest] = useState(0);
  const attempts = usePendingMessages((s) => s.attempts);
  const channelAttempts = useMemo(
    () =>
      Object.values(attempts).filter(
        (attempt) => attempt.channelId === channelId,
      ),
    [attempts, channelId],
  );
  const pending = usePendingMessages(
    (s) => s.byChannel[channelId] ?? nonePending,
  );
  const items = useMemo(
    () => visibleMessages(query.data?.pages ?? [], pending),
    [query.data?.pages, pending],
  );

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
  const muted = isMuted(
    readRows?.find((row) => row.channel_id === channelId),
    muteNow,
  );
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
          <div ref={panesRef} className="relative flex min-h-0 flex-1 flex-col">
            <div
              className={`flex min-h-0 flex-1 flex-col ${searching ? "invisible" : ""}`}
              inert={searching}
              aria-hidden={searching || undefined}
            >
              <MessageList
                key={searchScope}
                active={!searching}
                channelId={channelId}
                items={items}
                onAtLatest={setAtLatest}
                latestRequest={latestRequest}
                meId={user?.id}
                newAfter={newAfter ?? null}
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
                      canDelete={message.author.id === user?.id || canModerate}
                      onEdit={(content) =>
                        contextEdit.mutate({ id: message.id, content })
                      }
                      onDelete={async () => {
                        if (await confirmDelete(message, user?.id))
                          contextRemove.mutate(message.id);
                      }}
                    />
                  )}
                />
              </div>
            ) : null}
            {!searching && !atLatest && items.length > 0 ? (
              <button
                type="button"
                onClick={() => setLatestRequest((value) => value + 1)}
                className="lr-jump-latest"
              >
                <ArrowDownIcon size={15} />
                Zu den neuesten Nachrichten
              </button>
            ) : null}
          </div>
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
          channelId={channelId}
          channelName={channelName}
          canSend={canSend}
          canSendFiles={canSendFiles}
          mention={mention}
          author={
            user
              ? { id: user.id, name: user.name, avatar_url: user.avatar_url }
              : null
          }
          attempts={channelAttempts}
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
}: {
  active: boolean;
  channelId: string;
  items: Message[];
  meId: string | undefined;
  /** Last read message when the channel opened; "Neu" goes after it. */
  newAfter: string | null;
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
      const image = (message.attachments ?? []).some((a) =>
        isImageType(a.content_type),
      );
      const breaks =
        (dayBreak(items[index - 1], message) ? 40 : 0) +
        (index === newIndex ? 28 : 0);
      const gif = asGif(message.gif);
      if (gif)
        return (
          breaks +
          gifBox(gif).height +
          (isContinued(items[index - 1], message) ? 16 : 60)
        );
      // The image box is a fixed 240 px (chat.css), plus header and gaps.
      if (image)
        return breaks + (isContinued(items[index - 1], message) ? 256 : 300);
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
  }, [items, virtualizer, startOf, updateLatest, totalSize, latestRequest]);

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
      className="flex min-h-0 flex-1 flex-col overflow-y-auto"
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
      {ready && !loadError && items.length === 0 ? (
        <div className="lr-list-empty">
          Noch keine Nachrichten. Schreib die erste.
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
              className="absolute inset-x-0"
              style={{ top: row.start }}
            >
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
                onEdit={(content) => edit.mutate({ id: message.id, content })}
                onDelete={async () => {
                  if (await confirmDelete(message, meId))
                    remove.mutate(message.id);
                }}
              />
            </div>
          );
        })}
      </div>
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
  onEdit,
  onDelete,
}: {
  message: Message;
  continued: boolean;
  mine: boolean;
  canSend: boolean;
  canDelete: boolean;
  onEdit: (content: string) => void;
  onDelete: () => void;
}) {
  const pending = isPendingId(message.id);
  const { members, me } = useMentions();
  const mentioned = !mine && mentionsMe(message, me);
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
  const editButton = useRef<HTMLButtonElement>(null);
  const restoreEditFocus = useRef(false);
  useLayoutEffect(() => {
    if (!editing && restoreEditFocus.current) {
      restoreEditFocus.current = false;
      editButton.current?.focus({ preventScroll: true });
    }
  }, [editing]);
  const finishEditing = () => {
    restoreEditFocus.current = true;
    setEditing(false);
  };

  const startEdit = () => {
    restoreEditFocus.current = false;
    setDraft(fromWire(message.content, members));
    setEditing(true);
  };
  const cancelEdit = () => {
    finishEditing();
    setDraft(fromWire(message.content, members));
  };

  const when = formatWhen(message.created_at);
  const error = editing
    ? draft.length === 0
      ? message.attachments.length > 0
        ? null
        : validateContent(wireDraft)
      : validateContent(wireDraft)
    : null;
  const showDelete = canDelete && !pending && !editing;
  const gif = asGif(message.gif);
  const showEdit = mine && canSend && !pending && !editing && !gif;
  const canReact = canSend && !pending && !editing;
  const [reactionSlot, setReactionSlot] = useState<HTMLSpanElement | null>(
    null,
  );

  const actions =
    showEdit || showDelete || canReact ? (
      <span className="lr-message-actions">
        <span ref={setReactionSlot} className="lr-message-reaction-slot" />
        {showEdit ? (
          <IconButton
            buttonRef={editButton}
            label="Nachricht bearbeiten"
            onClick={startEdit}
          >
            <PencilIcon size={14} />
          </IconButton>
        ) : null}
        {showDelete ? (
          <IconButton label="Nachricht löschen" onClick={onDelete} danger>
            <TrashIcon size={14} />
          </IconButton>
        ) : null}
      </span>
    ) : null;

  if (continued && !editing) {
    return (
      <div
        tabIndex={-1}
        className={[
          "lr-message-row lr-message-continued group flex items-start gap-3",
          pending ? "opacity-60" : "",
          mentioned ? "is-mentioned" : "",
        ].join(" ")}
      >
        <span className="w-8 shrink-0" aria-hidden />
        <div className="min-w-0 flex-1">
          {message.content ? (
            <p className="lr-message-text">
              <MessageText content={message.content} />
              {message.edited_at ? (
                <span className="lr-message-edited">(bearbeitet)</span>
              ) : null}
            </p>
          ) : null}
          {gif ? <GifMessage gif={gif} /> : null}
          <AttachmentList attachments={message.attachments ?? []} />
          <ReactionBar
            message={message}
            canSend={canSend}
            addSlot={reactionSlot}
          />
        </div>
        {actions}
      </div>
    );
  }

  return (
    <div
      tabIndex={-1}
      className={[
        "lr-message-row group flex items-start gap-3",
        pending ? "opacity-60" : "",
        mentioned ? "is-mentioned" : "",
      ].join(" ")}
    >
      <Avatar name={message.author.name} url={message.author.avatar_url} />
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-2">
          <span className="lr-message-author">{message.author.name}</span>
          <time dateTime={message.created_at} className="lr-message-time">
            {when}
          </time>
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
          <p className="lr-message-text">
            <MessageText content={message.content} />
            {message.edited_at ? (
              <span className="lr-message-edited">(bearbeitet)</span>
            ) : null}
          </p>
        ) : null}
        {!editing && gif ? <GifMessage gif={gif} /> : null}
        {!editing ? (
          <AttachmentList attachments={message.attachments ?? []} />
        ) : null}
        {!editing && (
          <ReactionBar
            message={message}
            canSend={canSend}
            addSlot={reactionSlot}
          />
        )}
      </div>
      {actions}
    </div>
  );
}

function Composer({
  channelId,
  channelName,
  canSend,
  canSendFiles,
  mention,
  author,
  attempts,
  onDraftChange,
  onDraftStop,
}: {
  channelId: string;
  channelName: string;
  canSend: boolean;
  canSendFiles: boolean;
  mention: "#" | "@";
  author: { id: string; name: string; avatar_url: string | null } | null;
  attempts: SendAttempt[];
  onDraftChange?: (value: string) => void;
  onDraftStop?: () => void;
}) {
  const send = useSendMessage(
    channelId,
    author ?? { id: "", name: "", avatar_url: null },
  );
  const [savedDraft, updateDraft, preview] = useChatDraft(
    author?.id ?? "",
    channelId,
  );
  const draft = savedDraft.text;
  const file = savedDraft.file;
  const setDraft = (text: string) => updateDraft({ text });
  const setFile = (file: File | null) => updateDraft({ file });
  const [fileError, setFileError] = useState<string | null>(null);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [gifOpen, setGifOpen] = useState(false);
  const gifs = useGifStatus();
  const gifsEnabled = gifs.data?.enabled === true;
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
    Boolean(file && !canSendFiles) ||
    (emptyText && !file);

  const pickFile = (next: File | null) => {
    setFileError(null);
    if (!next) {
      setFile(null);
      return;
    }
    const invalid = validateAttachment(next);
    if (invalid) {
      setFile(null);
      setFileError(fieldMessage(invalid.field, invalid.code));
      return;
    }
    setFile(next);
  };

  // Files dropped anywhere on the conversation attach like the paperclip.
  const [dropping, setDropping] = useState(false);
  const attachDropped = useEffectEvent((next: File) => {
    pickFile(next);
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
      const file = droppedFile(event.dataTransfer);
      if (file) attachDropped(file);
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
    const attached = file;
    send.mutate({ content: text, file: attached ?? undefined });
    setDraft("");
    pickFile(null);
    onDraftStop?.();
    if (keepKeyboard.current) composerInput.current?.focus();
    keepKeyboard.current = false;
  };

  const sendGif = (pick: GifPick) => {
    setGifOpen(false);
    if (!canSend || !author) return;
    send.mutate({ content: "", gif: pick });
    composerInput.current?.focus();
  };

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
      {attempts
        .filter((attempt) => attempt.status !== "sending")
        .map((attempt) => (
          <div key={attempt.id} role="alert" className="lr-send-failure">
            <p>
              {attempt.error}{" "}
              {attempt.status === "uncertain"
                ? "Die Nachricht kann bereits gespeichert sein. Prüfe den Verlauf vor erneutem Senden."
                : "Text und Datei bleiben für dich erhalten."}
            </p>
            <p className="lr-send-failure-content">
              {attempt.gif ? (
                gifLabel(attempt.gif.gif)
              ) : (
                <MessageText content={attempt.content} />
              )}
            </p>
            {attempt.file ? <p>{attempt.file.name}</p> : null}
            <div className="lr-send-failure-actions">
              {attempt.status === "failed" ? (
                <button
                  type="button"
                  disabled={Boolean(attempt.file && !canSendFiles)}
                  onClick={() =>
                    send.mutate({
                      content: attempt.content,
                      attemptId: attempt.id,
                    })
                  }
                >
                  Sendung wiederholen
                </button>
              ) : null}
              {attempt.gif ? null : (
                <button
                  type="button"
                  onClick={() => {
                    setDraft(fromWire(attempt.content, members));
                    pickFile(attempt.file ?? null);
                    discardAttempt(attempt.id);
                    composerInput.current?.focus();
                  }}
                >
                  Entwurf übernehmen
                </button>
              )}
              <button type="button" onClick={() => discardAttempt(attempt.id)}>
                Verwerfen
              </button>
            </div>
          </div>
        ))}
      <label className="sr-only" htmlFor={`compose-${channelId}`}>
        Nachricht in {mention}
        {channelName}
      </label>
      <div className="lr-composer-field">
        {dropping ? (
          <p className="lr-composer-drop" aria-hidden="true">
            Loslassen zum Anhängen
          </p>
        ) : null}
        {file ? (
          <div className="lr-composer-attachment">
            {preview ? <img src={preview} alt="" /> : null}
            <div>
              <p>{file.name}</p>
              <p>{(file.size / 1024).toFixed(0)} KB</p>
            </div>
            <button
              type="button"
              onClick={() => pickFile(null)}
              aria-label="Anhang entfernen"
              title="Anhang entfernen"
            >
              <CloseIcon size={14} />
            </button>
          </div>
        ) : null}
        {suggestions.list}
        <div className="lr-composer-row">
          {canSendFiles ? (
            <>
              <input
                ref={fileInput}
                type="file"
                accept={ALLOWED_TYPES.join(",")}
                className="sr-only"
                onChange={(event) => {
                  const selected = event.target.files?.[0] ?? null;
                  pickFile(selected);
                  event.target.value = "";
                  if (selected) composerInput.current?.focus();
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
              const pasted = pastedFile(event.clipboardData);
              if (!pasted) return;
              event.preventDefault();
              pickFile(pasted);
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
      {remaining < 200 || error || fileError ? (
        <div className="lr-composer-status">
          {remaining < 200 ? (
            <span className={remaining < 0 ? "lr-field-error" : undefined}>
              {remaining}
            </span>
          ) : null}
          {error ? (
            <span className="lr-field-error">
              {fieldMessage("content", error)}
            </span>
          ) : null}
          {fileError ? (
            <span className="lr-field-error">{fileError}</span>
          ) : null}
        </div>
      ) : null}
    </form>
  );
}

function GifMessage({ gif }: { gif: Gif }) {
  return <GifMessageButton gif={gif} reduced={useReducedMotion()} />;
}

function AttachmentList({ attachments }: { attachments: Attachment[] }) {
  const openImage = useContext(OpenImage);
  const files = asAttachmentList(attachments);
  if (files.length === 0) return null;
  return (
    <div className="lr-attachments">
      {files.map((attachment) => {
        const src = attachment.preview_url ?? attachmentUrl(attachment.id);
        if (isImageType(attachment.content_type)) {
          return (
            <a
              key={attachment.id}
              href={src}
              // The link stays for "open in new tab"; a click shows the
              // picture here, also where no new tab exists (desktop app,
              // installed web app).
              onClick={(event) => {
                if (!isPlainClick(event)) return;
                event.preventDefault();
                openImage({
                  id: attachment.id,
                  src,
                  filename: attachment.filename,
                  download: attachmentDownloadUrl(attachment.id),
                });
              }}
              aria-label={`${attachment.filename} vergrößern`}
              className="lr-attachment-image"
            >
              {/* A fixed-height box: the row has its final height before the
                  picture arrives, so loading never moves the list. */}
              <img
                src={src}
                alt={attachment.filename}
                decoding="async"
                loading="lazy"
              />
            </a>
          );
        }
        return (
          <a
            key={attachment.id}
            // Saved, never opened: an installed app has no back button to
            // return from a file, and after MinIO's redirect the browser
            // ignores `download` and would leave the app (and a call).
            href={attachmentDownloadUrl(attachment.id)}
            download={attachment.filename}
            className="lr-attachment-file"
          >
            <PaperclipIcon size={15} />
            <span>{attachment.filename}</span>
            <span>{(attachment.size / 1024).toFixed(0)} KB</span>
          </a>
        );
      })}
    </div>
  );
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

function IconButton({
  buttonRef,
  label,
  onClick,
  children,
  danger = false,
}: {
  buttonRef?: Ref<HTMLButtonElement>;
  label: string;
  onClick: () => void;
  children: ReactNode;
  danger?: boolean;
}) {
  return (
    <button
      ref={buttonRef}
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={`lr-message-action${danger ? " is-danger" : ""}`}
    >
      {children}
    </button>
  );
}
