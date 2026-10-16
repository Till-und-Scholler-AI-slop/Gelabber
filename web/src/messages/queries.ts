// TanStack Query layer for a channel's messages.
//
// Feel rule: a send lands in the pending overlay *before* the request
// leaves, so the row is on screen in the same frame (< 50 ms). An error
// drops that row (no ghost). Edit/delete patch the cache first and roll
// back on failure. History is an infinite query: first page = latest,
// `fetchNextPage` walks older via the id cursor.

import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
  infiniteQueryOptions,
  replaceEqualDeep,
} from "@tanstack/react-query";

import { ApiError } from "../api/client.ts";
import { scopeGeneration, stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { notifyError } from "../components/toasts.ts";
import { randomUuid } from "../lib/uuid.ts";
import { isPendingId } from "../servers/queries.ts";
import type { GifPick } from "../gifs/api.ts";
import { refreshChatWorkflows } from "./readState.ts";
import * as remote from "./api.ts";
import {
  applyMessageChanges,
  combineMessageChange,
  encodeCursor,
  newerMessage,
  olderCursor,
  stampOlder,
  type MessageChange,
} from "./pages.ts";
import {
  addPending,
  confirmPending,
  forgetAttempt,
  saveAttempt,
  type SendAttempt,
  usePendingMessages,
} from "./pending.ts";
import {
  PAGE_SIZE,
  inferContentType,
  isImageType,
  normaliseContent,
} from "./rules.ts";
import type {
  Attachment,
  Message,
  MessageAuthor,
  MessagePage,
} from "./types.ts";
import { asAttachmentList, asReactionList } from "./types.ts";

export const messageKeys = {
  channel: (userId: string, generation: number, channelId: string) =>
    ["user", userId, generation, "messages", channelId] as const,
};

type Cache = InfiniteData<MessagePage, string | undefined>;

const STALE_MS = 30_000;

function tmpId(): string {
  return `tmp:${randomUuid()}`;
}

function now(): string {
  return new Date().toISOString();
}

type ReadChanges = { changes: Map<string, MessageChange>; manual: boolean };
const manualPatches = new WeakSet<QueryClient>();
const journals = new WeakMap<QueryClient, Map<string, ReadChanges>>();
type EntityScope = {
  scope: string;
  channels: Map<string, Map<string, MessageChange>>;
};
// Floors outlive a REST read so a stale page or a replayed event cannot
// overwrite a newer row or revive a delete. A channel with a cached query
// keeps its most recently touched rows; without one only compact tombstones
// remain, since delivery follows the commit and its next GET is never older
// than an event already seen.
export const ENTITY_LIMIT = 500;
const entities = new WeakMap<QueryClient, EntityScope>();
const entityCleanup = new WeakSet<QueryClient>();
function channelEntities(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
): Map<string, MessageChange> {
  if (!entityCleanup.has(client)) {
    entityCleanup.add(client);
    client.getQueryCache().subscribe((event) => {
      const state = entities.get(client);
      if (event.type !== "removed" || !state) return;
      // Scope is stored as a tuple; compare the current identity explicitly.
      const [owner, epoch] = JSON.parse(state.scope) as [string, number];
      if (!stampHolds({ userId: owner, generation: epoch })) {
        entities.delete(client);
        rowVersions.delete(client);
        return;
      }
      const key = event.query.queryKey;
      if (key[3] !== "messages" || key[1] !== owner || key[2] !== epoch) return;
      const rows = state.channels.get(String(key[4]));
      if (!rows) return;
      for (const [id, change] of rows) if (change.message) rows.delete(id);
      if (rows.size === 0) state.channels.delete(String(key[4]));
    });
  }
  const scope = JSON.stringify([userId, generation]);
  let state = entities.get(client);
  if (!state || state.scope !== scope) {
    state = { scope, channels: new Map() };
    entities.set(client, state);
  }
  let channel = state.channels.get(channelId);
  if (!channel) {
    channel = new Map();
    state.channels.set(channelId, channel);
  }
  return channel;
}

/** Re-inserted on every touch, so the first key is the least recent one. */
function keepEntity(
  rows: Map<string, MessageChange>,
  change: MessageChange,
): void {
  rows.delete(change.id);
  // A retained entity is a floor/overlay, not evidence of complete history.
  rows.set(change.id, { ...change, created: false });
  for (const id of rows.keys()) {
    if (rows.size <= ENTITY_LIMIT) break;
    rows.delete(id);
  }
}

/** Rows retained across all channels of this client's current scope. */
export function retainedEntityCount(client: QueryClient): number {
  let count = 0;
  for (const rows of entities.get(client)?.channels.values() ?? [])
    count += rows.size;
  return count;
}

function retainChange(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  incoming: MessageChange,
  cached: boolean,
): MessageChange {
  const rows = channelEntities(client, userId, generation, channelId);
  const previous = rows.get(incoming.id);
  const change = combineMessageChange(previous, incoming);
  if (cached || change.message === null) keepEntity(rows, change);
  return { ...change, created: incoming.created && change.message !== null };
}

/** Changes live only for the lifetime of a REST read, including all its pages. */
function readJournals(client: QueryClient): Map<string, ReadChanges> {
  const existing = journals.get(client);
  if (existing) return existing;
  const reads = new Map<string, ReadChanges>();
  journals.set(client, reads);
  client.getQueryCache().subscribe((event) => {
    if (event.query.queryKey[3] !== "messages") return;
    const hash = event.query.queryHash;
    if (event.type === "removed") reads.delete(hash);
    if (event.type !== "updated") return;
    if (event.action.type === "fetch")
      reads.set(hash, { changes: new Map(), manual: false });
    if (
      event.action.type === "error" ||
      (event.action.type === "success" && !event.action.manual)
    )
      reads.delete(hash);
  });
  return reads;
}

function recordChange(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  change: MessageChange,
): MessageChange {
  const query = client.getQueryCache().find({
    queryKey: messageKeys.channel(userId, generation, channelId),
    exact: true,
  });
  change = retainChange(client, userId, generation, channelId, change, !!query);
  if (!query) return change;
  const read = readJournals(client).get(query.queryHash);
  if (!read) return change;
  const previous = read.changes.get(change.id);
  const merged = combineMessageChange(previous, change);
  read.changes.set(change.id, merged);
  return merged;
}

function patchPages(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  update: (pages: MessagePage[]) => MessagePage[],
): void {
  const key = messageKeys.channel(userId, generation, channelId);
  const query = client.getQueryCache().find({ queryKey: key, exact: true });
  const previous = query?.state;
  const read = query && readJournals(client).get(query.queryHash);
  const observed = (query?.getObserversCount() ?? 0) > 0;
  if (read) read.manual = true;
  manualPatches.add(client);
  try {
    client.setQueryData<Cache>(
      key,
      (current) => {
        // An event is a delta, never proof that an unopened history is complete.
        if (!current) return undefined;
        return capLatestPage(
          { ...current, pages: update(current.pages) },
          observed,
        );
      },
      { updatedAt: previous?.dataUpdatedAt },
    );
  } finally {
    manualPatches.delete(client);
    if (read) read.manual = false;
  }
  if (previous?.isInvalidated) {
    void client.invalidateQueries({
      queryKey: key,
      exact: true,
      refetchType: "none",
    });
  }
}

/**
 * Live creates land in the newest page; keep it at one page of rows. Off
 * screen the oldest rows (and every older page, which they would otherwise
 * split from) are dropped and paged back in on scroll. On screen they move
 * one page down so the reader's history stays put: into the older page that
 * is loaded, or else into a new one (dropping them would shift everything a
 * reader scrolled up to by one row per message).
 */
function capLatestPage(data: Cache, observed: boolean): Cache {
  const [latest, next, ...rest] = data.pages;
  if (!latest || latest.messages.length <= PAGE_SIZE) return data;
  const cut = latest.messages.length - PAGE_SIZE;
  const kept = latest.messages.slice(cut);
  const head = {
    ...latest,
    messages: kept,
    has_more: true,
    older: encodeCursor(kept[0]!),
  };
  if (observed && !next)
    return {
      pages: [head, { ...latest, messages: latest.messages.slice(0, cut) }],
      pageParams: [...data.pageParams.slice(0, 1), head.older],
    };
  if (observed && next)
    return {
      ...data,
      pages: [
        head,
        {
          ...next,
          messages: [...next.messages, ...latest.messages.slice(0, cut)],
        },
        ...rest,
      ],
    };
  return { pages: [head], pageParams: data.pageParams.slice(0, 1) };
}

function mapMessages(
  pages: MessagePage[],
  update: (message: Message) => Message | null,
): MessagePage[] {
  return pages.map((page) => ({
    ...page,
    messages: page.messages
      .map((message) => update(message))
      .filter((message): message is Message => message !== null),
  }));
}

export function useMessages(channelId: string | undefined, enabled: boolean) {
  const client = useQueryClient();
  const userId = useSession((state) => state.user)?.id ?? "";
  const generation = scopeGeneration();
  return useInfiniteQuery({
    ...messageQueryOptions(client, userId, generation, channelId ?? ""),
    enabled: userId.length > 0 && Boolean(channelId) && enabled,
  });
}

/** Shared by the hook and recovery tests using the real infinite-query engine. */
export function messageQueryOptions(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
) {
  const reads = readJournals(client);
  const key = messageKeys.channel(userId, generation, channelId);
  return infiniteQueryOptions({
    queryKey: key,
    queryFn: async ({ pageParam, signal }) =>
      stampOlder(
        await remote.listMessages(
          channelId,
          { before: pageParam, limit: PAGE_SIZE },
          signal,
        ),
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => olderCursor(page),
    structuralSharing: (previous, incoming) => {
      const query = client.getQueryCache().find({ queryKey: key, exact: true });
      const read = query && reads.get(query.queryHash);
      let data = incoming as Cache;
      if (!manualPatches.has(client)) {
        const floors = channelEntities(client, userId, generation, channelId);
        for (const page of data.pages)
          for (const message of page.messages)
            keepEntity(
              floors,
              combineMessageChange(floors.get(message.id), {
                id: message.id,
                message,
                created: false,
              }),
            );
        data = {
          ...data,
          pages: applyMessageChanges(data.pages, floors.values()),
        };
      }
      const merged =
        read && !read.manual && read.changes.size > 0
          ? {
              ...data,
              pages: applyMessageChanges(data.pages, read.changes.values()),
            }
          : data;
      return replaceEqualDeep(previous, merged);
    },
    staleTime: STALE_MS,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

export type SendInput = {
  content: string;
  file?: File;
  /** Sent alone: no text, no file. */
  gif?: GifPick;
};

function localAttachment(file: File): Attachment {
  const contentType = inferContentType(file);
  return {
    id: `tmp:${randomUuid()}`,
    filename: file.name.replace(/^.*[/\\]/, ""),
    content_type: contentType,
    size: file.size,
    preview_url: isImageType(contentType)
      ? URL.createObjectURL(file)
      : undefined,
  };
}

function requireAttempt(attempt: SendAttempt): void {
  if (!stampHolds(attempt.stamp) || attempt.controller.signal.aborted)
    throw new DOMException("Session changed", "AbortError");
}

/** The attempt owns its file/status even when the mutation observer unmounts. */
export async function sendMessageAttempt(
  channelId: string,
  author: MessageAuthor,
  input: SendInput & { attemptId?: string },
): Promise<Message> {
  let attempt = input.attemptId
    ? usePendingMessages.getState().attempts[input.attemptId]
    : undefined;
  if (input.attemptId && (!attempt || attempt.status === "sending"))
    throw new Error(
      "Dieser Sendeversuch läuft bereits oder ist nicht mehr vorhanden.",
    );
  if (attempt?.status === "uncertain")
    throw new Error(
      "Speicherstatus unbekannt. Bitte zuerst den Nachrichtenverlauf prüfen.",
    );
  if (!attempt) {
    const stamp = takeStamp();
    if (!stamp || stamp.userId !== author.id)
      throw new DOMException("Session changed", "AbortError");
    const id = tmpId();
    attempt = {
      id,
      channelId,
      stamp,
      content: input.gif ? "" : normaliseContent(input.content),
      file: input.gif ? undefined : input.file,
      gif: input.gif,
      status: "sending",
      stage: input.file && !input.gif ? "presign" : "bind",
      controller: new AbortController(),
    };
    addPending(channelId, {
      id,
      channel_id: channelId,
      author,
      content: attempt.content,
      created_at: now(),
      edited_at: null,
      attachments: attempt.file ? [localAttachment(attempt.file)] : [],
      gif: input.gif?.gif ?? null,
    });
  }
  requireAttempt(attempt);
  if (attempt.channelId !== channelId)
    throw new Error("Falscher Kanal für den Sendeversuch.");
  attempt = { ...attempt, status: "sending", error: undefined };
  saveAttempt(attempt);
  try {
    if (attempt.file && !attempt.uploadedId) {
      const file = attempt.file;
      attempt = { ...attempt, stage: "presign" };
      saveAttempt(attempt);
      const presign = await remote.presignAttachment(channelId, {
        filename: file.name,
        content_type: inferContentType(file),
        size: file.size,
      });
      requireAttempt(attempt);
      attempt = { ...attempt, stage: "upload" };
      saveAttempt(attempt);
      await remote.putPresigned(
        presign.upload_url,
        file,
        presign.headers,
        attempt.controller.signal,
      );
      requireAttempt(attempt);
      attempt = { ...attempt, uploadedId: presign.id };
    }
    requireAttempt(attempt);
    attempt = { ...attempt, stage: "bind" };
    saveAttempt(attempt);
    const message = await remote.createMessage(
      channelId,
      attempt.content,
      attempt.uploadedId ? [attempt.uploadedId] : [],
      attempt.gif,
    );
    requireAttempt(attempt);
    confirmPending(channelId, attempt.id, message);
    forgetAttempt(attempt.id);
    return message;
  } catch (error) {
    if (stampHolds(attempt.stamp)) {
      // No idempotency key exists yet. A missing bind response cannot safely
      // authorize another POST; retain the draft and require history inspection.
      const uncertain =
        attempt.stage === "bind" &&
        (!(error instanceof ApiError) ||
          error.code === "network" ||
          error.status === 0 ||
          error.status === 408 ||
          error.status >= 500);
      const labels = {
        presign: "Dateifreigabe",
        upload: "Dateiübertragung",
        bind: "Nachricht speichern",
      };
      saveAttempt({
        ...attempt,
        status: uncertain ? "uncertain" : "failed",
        error: `${labels[attempt.stage]} fehlgeschlagen.`,
      });
      notifyError(error);
    }
    throw error;
  }
}

export function useSendMessage(channelId: string, author: MessageAuthor) {
  const client = useQueryClient();
  return useMutation({
    onMutate: () => takeStamp(),
    onSettled: (_result, _error, _input, stamp) => {
      if (stampHolds(stamp))
        void refreshChatWorkflows(client, stamp, channelId);
    },
    mutationFn: (input: SendInput & { attemptId?: string }) =>
      sendMessageAttempt(channelId, author, input),
    retry: false,
  });
}

// A per-row token also covers a delete whose optimistic row is absent.
const rowVersions = new WeakMap<
  QueryClient,
  { scope: string; versions: Map<string, number> }
>();
function rowVersion(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  id: string,
  bump = false,
): number {
  const scope = JSON.stringify([userId, generation]);
  let state = rowVersions.get(client);
  if (!state || state.scope !== scope) {
    state = { scope, versions: new Map() };
    rowVersions.set(client, state);
  }
  const versions = state.versions;
  const key = JSON.stringify([channelId, id]);
  const version = (versions.get(key) ?? 0) + (bump ? 1 : 0);
  if (bump) versions.set(key, version);
  return version;
}
/** A live change only needs to outdate the token of an in-flight mutation. */
function bumpRowVersion(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  id: string,
): void {
  const state = rowVersions.get(client);
  if (state?.scope !== JSON.stringify([userId, generation])) return;
  const key = JSON.stringify([channelId, id]);
  const version = state.versions.get(key);
  if (version !== undefined) state.versions.set(key, version + 1);
}

export function editMessageOptions(client: QueryClient, channelId: string) {
  return {
    mutationFn: ({ id, content }: { id: string; content: string }) =>
      remote.updateMessage(id, normaliseContent(content)),
    onMutate: ({ id, content }: { id: string; content: string }) => {
      const stamp = takeStamp();
      if (!stamp) throw new DOMException("Session changed", "AbortError");
      const cache = client.getQueryData<Cache>(
        messageKeys.channel(stamp.userId, stamp.generation, channelId),
      );
      const previous = cache?.pages
        .flatMap((page) => page.messages)
        .find((row) => row.id === id);
      const optimistic = previous && {
        ...previous,
        content: normaliseContent(content),
        edited_at: now(),
      };
      const version = rowVersion(
        client,
        stamp.userId,
        stamp.generation,
        channelId,
        id,
        true,
      );
      if (optimistic)
        patchPages(client, stamp.userId, stamp.generation, channelId, (pages) =>
          mapMessages(pages, (row) => (row.id === id ? optimistic : row)),
        );
      return { ...stamp, id, previous, optimistic, version };
    },
    onError: (
      error: Error,
      _vars: { id: string; content: string },
      ctx: EditContext | undefined,
    ) => {
      if (!stampHolds(ctx)) return;
      if (
        rowVersion(client, ctx.userId, ctx.generation, channelId, ctx.id) ===
          ctx.version &&
        ctx.previous
      )
        patchPages(client, ctx.userId, ctx.generation, channelId, (pages) =>
          mapMessages(pages, (row) =>
            row.id === ctx.id &&
            row.content === ctx.optimistic?.content &&
            row.edited_at === ctx.optimistic.edited_at
              ? ctx.previous!
              : row,
          ),
        );
      void client.invalidateQueries({
        queryKey: messageKeys.channel(ctx.userId, ctx.generation, channelId),
        exact: true,
      });
      notifyError(error);
    },
    onSuccess: (
      message: Message,
      _vars: { id: string; content: string },
      ctx: EditContext | undefined,
    ) => {
      if (!stampHolds(ctx)) return;
      const versionMatches =
        rowVersion(client, ctx.userId, ctx.generation, channelId, ctx.id) ===
        ctx.version;
      // Legacy replies cannot order themselves against a newer mutation/event.
      if (message.revision === undefined && !versionMatches) return;
      const current = client
        .getQueryData<Cache>(
          messageKeys.channel(ctx.userId, ctx.generation, channelId),
        )
        ?.pages.flatMap((page) => page.messages)
        .find((row) => row.id === ctx.id);
      if (message.revision === undefined && !current) return;
      const ownsOptimistic =
        versionMatches &&
        current !== undefined &&
        current.content === ctx.optimistic?.content &&
        current.edited_at === ctx.optimistic.edited_at;
      // REST may replace the optimistic row with either an older or newer
      // snapshot. Compare canonical values, never the local optimistic clock.
      const canonical =
        message.revision === undefined && current && !ownsOptimistic
          ? newerMessage(current, message)
          : message;
      const change = recordChange(
        client,
        ctx.userId,
        ctx.generation,
        channelId,
        { id: message.id, message: canonical, created: false },
      );
      patchPages(client, ctx.userId, ctx.generation, channelId, (pages) =>
        ownsOptimistic
          ? mapMessages(pages, (row) =>
              row.id === ctx.id ? change.message : row,
            )
          : applyMessageChanges(pages, [change]),
      );
    },
  };
}
type EditContext = import("../auth/scope.ts").ScopeStamp & {
  id: string;
  previous?: Message;
  optimistic?: Message;
  version: number;
};

export function useEditMessage(channelId: string) {
  const client = useQueryClient();
  return useMutation({
    ...editMessageOptions(client, channelId),
    onSettled: (_result, _error, _input, stamp) => {
      if (stampHolds(stamp))
        void refreshChatWorkflows(client, stamp, channelId);
    },
  });
}

function isMessage(value: unknown): value is Message {
  if (value === null || typeof value !== "object") return false;
  const row = value as Message;
  return (
    typeof row.id === "string" &&
    typeof row.channel_id === "string" &&
    typeof row.content === "string" &&
    typeof row.created_at === "string" &&
    row.author !== null &&
    typeof row.author === "object" &&
    typeof row.author.id === "string"
  );
}

/** WS create: append if the row is not already in a page. */
export function applyMessageCreated(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  message: Message,
): void {
  bumpRowVersion(client, userId, generation, channelId, message.id);
  const change = recordChange(client, userId, generation, channelId, {
    id: message.id,
    message,
    created: true,
  });
  patchPages(client, userId, generation, channelId, (pages) =>
    applyMessageChanges(pages, [change]),
  );
}

/** WS edit: replace the row in place. */
export function applyMessageEdited(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  message: Message,
): void {
  bumpRowVersion(client, userId, generation, channelId, message.id);
  const change = recordChange(client, userId, generation, channelId, {
    id: message.id,
    message,
    created: false,
  });
  patchPages(client, userId, generation, channelId, (pages) =>
    applyMessageChanges(pages, [change]),
  );
}

/** WS / optimistic delete: drop the row, keep page cursors. */
export function applyMessageDeleted(
  client: QueryClient,
  userId: string,
  generation: number,
  channelId: string,
  messageId: string,
  revision?: number,
): void {
  bumpRowVersion(client, userId, generation, channelId, messageId);
  const change = recordChange(client, userId, generation, channelId, {
    id: messageId,
    message: null,
    created: false,
    revision,
  });
  patchPages(client, userId, generation, channelId, (pages) =>
    applyMessageChanges(pages, [change]),
  );
}

export function applyChannelEvent(
  client: QueryClient,
  userId: string,
  generation: number,
  event: {
    t: "c" | "e" | "d";
    c?: string;
    i?: string;
    d?: unknown;
    r?: number;
  },
): void {
  const channelId = event.c;
  if (!channelId || userId.length === 0) return;
  if (event.t === "d" && event.i) {
    applyMessageDeleted(
      client,
      userId,
      generation,
      channelId,
      event.i,
      event.r,
    );
    return;
  }
  if ((event.t === "c" || event.t === "e") && isMessage(event.d)) {
    const message = {
      ...event.d,
      ...(event.r !== undefined ? { revision: event.r } : {}),
      attachments: asAttachmentList(event.d.attachments),
      ...(event.d.reactions === undefined
        ? {}
        : { reactions: asReactionList(event.d.reactions) }),
    };
    if (event.t === "c")
      applyMessageCreated(client, userId, generation, channelId, message);
    else applyMessageEdited(client, userId, generation, channelId, message);
  }
}

type DeleteContext = import("../auth/scope.ts").ScopeStamp & {
  id: string;
  previous?: Message;
  page: number;
  version: number;
};
export function deleteMessageOptions(client: QueryClient, channelId: string) {
  return {
    mutationFn: (id: string) => remote.deleteMessage(id),
    onSuccess: (_result: null, id: string, ctx: DeleteContext | undefined) => {
      if (stampHolds(ctx))
        applyMessageDeleted(client, ctx.userId, ctx.generation, channelId, id);
    },
    onMutate: (id: string) => {
      const stamp = takeStamp();
      if (!stamp) throw new DOMException("Session changed", "AbortError");
      const cache = client.getQueryData<Cache>(
        messageKeys.channel(stamp.userId, stamp.generation, channelId),
      );
      const page =
        cache?.pages.findIndex((page) =>
          page.messages.some((row) => row.id === id),
        ) ?? -1;
      const previous = cache?.pages[page]?.messages.find(
        (row) => row.id === id,
      );
      const version = rowVersion(
        client,
        stamp.userId,
        stamp.generation,
        channelId,
        id,
        true,
      );
      patchPages(client, stamp.userId, stamp.generation, channelId, (pages) =>
        mapMessages(pages, (row) => (row.id === id ? null : row)),
      );
      return { ...stamp, id, previous, page, version };
    },
    onError: (error: Error, _id: string, ctx: DeleteContext | undefined) => {
      if (!stampHolds(ctx)) return;
      if (
        ctx.previous &&
        rowVersion(client, ctx.userId, ctx.generation, channelId, ctx.id) ===
          ctx.version
      )
        patchPages(client, ctx.userId, ctx.generation, channelId, (pages) => {
          if (
            pages.some((page) => page.messages.some((row) => row.id === ctx.id))
          )
            return pages;
          return pages.map((page, index) =>
            index === Math.min(ctx.page, pages.length - 1)
              ? {
                  ...page,
                  messages: [...page.messages, ctx.previous!].sort(
                    (a, b) =>
                      a.created_at.localeCompare(b.created_at) ||
                      a.id.localeCompare(b.id),
                  ),
                }
              : page,
          );
        });
      void client.invalidateQueries({
        queryKey: messageKeys.channel(ctx.userId, ctx.generation, channelId),
        exact: true,
      });
      notifyError(error);
    },
  };
}
export function useDeleteMessage(channelId: string) {
  const client = useQueryClient();
  return useMutation({
    ...deleteMessageOptions(client, channelId),
    onSettled: (_result, _error, _input, stamp) => {
      if (stampHolds(stamp))
        void refreshChatWorkflows(client, stamp, channelId);
    },
  });
}

export { isPendingId };
