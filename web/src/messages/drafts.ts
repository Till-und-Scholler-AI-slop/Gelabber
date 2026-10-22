// Unsent messages per account and conversation. The text survives a restart
// of the app for a week; attached files only live in this tab.
import { useCallback, useState, useSyncExternalStore } from "react";
import { inferContentType, isImageType } from "./rules.ts";
import { scopeGeneration, stampHolds } from "../auth/scope.ts";

export type Draft = { text: string; files: File[] };
const memory = new Map<string, Draft>();
/** Object URLs of a draft's pictures, by file. */
const previews = new Map<string, Map<File, string>>();
const none: ReadonlyMap<File, string> = new Map();
const prefix = "gelabber:chat-draft:";
/** A stored text older than this is dropped instead of restored. */
export const DRAFT_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
let account: string | null = null;
const listeners = new Set<() => void>();
const key = (userId: string, channelId: string) =>
  `${prefix}${encodeURIComponent(userId)}:${encodeURIComponent(channelId)}`;

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

function changed(): void {
  for (const listener of listeners) listener();
}

/** The stored text, or "" when there is none, it expired or is unreadable. */
function storedText(value: string | null, now: number): string {
  if (!value) return "";
  try {
    const parsed: unknown = JSON.parse(value);
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      "text" in parsed &&
      "at" in parsed &&
      typeof parsed.text === "string" &&
      typeof parsed.at === "number" &&
      now - parsed.at < DRAFT_MAX_AGE_MS
    )
      return parsed.text;
  } catch {
    /* Not ours, or damaged: no draft. */
  }
  return "";
}

export function loadDraft(userId: string, channelId: string): Draft {
  const id = key(userId, channelId);
  const cached = memory.get(id);
  if (cached) return cached;
  let text = "";
  try {
    const value = storage()?.getItem(id) ?? null;
    text = storedText(value, Date.now());
    if (value !== null && !text) storage()?.removeItem(id);
  } catch {
    /* Storage quota/privacy. */
  }
  const draft: Draft = { text, files: [] };
  memory.set(id, draft);
  return draft;
}

/** Whether the conversation has an unsent text or file. */
export function hasDraft(userId: string, channelId: string): boolean {
  if (!userId) return false;
  const draft = loadDraft(userId, channelId);
  return draft.text.trim().length > 0 || draft.files.length > 0;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** `hasDraft`, kept current while the user types elsewhere. */
export function useHasDraft(userId: string, channelId: string): boolean {
  return useSyncExternalStore(
    subscribe,
    () => hasDraft(userId, channelId),
    () => false,
  );
}

export function saveDraft(
  userId: string,
  channelId: string,
  draft: Draft,
): void {
  const id = key(userId, channelId);
  const previous = previews.get(id) ?? new Map<File, string>();
  const next = new Map<File, string>();
  for (const file of draft.files) {
    const url = previous.get(file);
    if (url) next.set(file, url);
    else if (isImageType(inferContentType(file)))
      next.set(file, URL.createObjectURL(file));
  }
  for (const [file, url] of previous)
    if (!next.has(file)) URL.revokeObjectURL(url);
  if (next.size > 0) previews.set(id, next);
  else previews.delete(id);
  if (!draft.text && draft.files.length === 0) memory.delete(id);
  else memory.set(id, draft);
  try {
    if (draft.text)
      storage()?.setItem(
        id,
        JSON.stringify({ text: draft.text, at: Date.now() }),
      );
    else storage()?.removeItem(id);
  } catch {
    /* The in-memory draft remains available. */
  }
  changed();
}

/** Initial session bootstrap preserves stored drafts; logout/account change
 * releases them. Expired ones go at the same time. */
export function bindDraftAccount(next: string | null): void {
  const target = storage();
  const retain = next === account ? next : account === null ? next : null;
  const retainedPrefix = retain
    ? `${prefix}${encodeURIComponent(retain)}:`
    : null;
  for (const id of memory.keys())
    if (!retainedPrefix || !id.startsWith(retainedPrefix)) memory.delete(id);
  for (const [id, urls] of previews) {
    if (!retainedPrefix || !id.startsWith(retainedPrefix)) {
      for (const url of urls.values()) URL.revokeObjectURL(url);
      previews.delete(id);
    }
  }
  const now = Date.now();
  try {
    for (const id of Object.keys(target ?? {})) {
      if (!id.startsWith(prefix)) continue;
      if (
        !retainedPrefix ||
        !id.startsWith(retainedPrefix) ||
        !storedText(target?.getItem(id) ?? null, now)
      )
        target?.removeItem(id);
    }
  } catch {
    /* Storage may be disabled. */
  }
  account = next;
  changed();
}

export function useChatDraft(userId: string, channelId: string) {
  const generation = scopeGeneration();
  const [draft, setLocal] = useState(() => loadDraft(userId, channelId));
  const update = useCallback(
    (patch: Partial<Draft>) => {
      if (!stampHolds({ userId, generation })) return;
      const next = { ...loadDraft(userId, channelId), ...patch };
      saveDraft(userId, channelId, next);
      setLocal(next);
    },
    [userId, generation, channelId],
  );
  return [draft, update, previews.get(key(userId, channelId)) ?? none] as const;
}
