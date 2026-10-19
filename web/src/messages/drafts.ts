import { useCallback, useState } from "react";
import { inferContentType, isImageType } from "./rules.ts";
import { scopeGeneration, stampHolds } from "../auth/scope.ts";

export type Draft = { text: string; files: File[] };
const memory = new Map<string, Draft>();
/** Object URLs of a draft's pictures, by file. */
const previews = new Map<string, Map<File, string>>();
const none: ReadonlyMap<File, string> = new Map();
const prefix = "gelabber:chat-draft:";
let account: string | null = null;
const key = (userId: string, channelId: string) =>
  `${prefix}${encodeURIComponent(userId)}:${encodeURIComponent(channelId)}`;

function storage(): Storage | undefined {
  try {
    return globalThis.sessionStorage;
  } catch {
    return undefined;
  }
}

export function loadDraft(userId: string, channelId: string): Draft {
  const id = key(userId, channelId);
  const cached = memory.get(id);
  if (cached) return cached;
  let text = "";
  try {
    text = storage()?.getItem(id) ?? "";
  } catch {
    /* Storage quota/privacy. */
  }
  const draft: Draft = { text, files: [] };
  memory.set(id, draft);
  return draft;
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
    if (draft.text) storage()?.setItem(id, draft.text);
    else storage()?.removeItem(id);
  } catch {
    /* The in-memory draft remains available. */
  }
}

/** Initial session bootstrap preserves reload drafts; logout/account change releases them. */
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
  try {
    for (const id of Object.keys(target ?? {})) {
      if (
        id.startsWith(prefix) &&
        (!retainedPrefix || !id.startsWith(retainedPrefix))
      )
        target?.removeItem(id);
    }
  } catch {
    /* Storage may be disabled. */
  }
  account = next;
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
