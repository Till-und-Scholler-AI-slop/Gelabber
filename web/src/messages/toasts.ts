// In-app toasts for chat creates in a channel the tab is not looking at.

import { create } from "zustand";

import {
  TOAST_HOLD_MS,
  TOAST_MAX,
  stackOnto,
  type StackableToast,
} from "./notify.ts";

export type MessageToast = {
  id: number;
  channelId: string;
  serverId: string;
  dm: boolean;
  channelLabel: string;
  author: string;
  /** The author's picture, if they have one. */
  authorAvatar?: string | null;
  /** The message mentions the user. */
  mentioned?: boolean;
  preview: string;
  count: number;
  at: number;
};

/** Why a toast is held on screen: the pointer is on it, or focus is in it. */
export type MessageToastHold = "hover" | "focus";

type MessageToastState = {
  toasts: MessageToast[];
  push: (input: Omit<MessageToast, "id" | "count" | "at">) => void;
  dismiss: (id: number) => void;
  /** Stops the countdown while someone reads or uses the toast. */
  hold: (id: number, reason: MessageToastHold) => void;
  /** Resumes it once nothing holds the toast any more. */
  release: (id: number, reason: MessageToastHold) => void;
};

/** Back from a hold, there is always a moment left to finish reading. */
export const RELEASE_MIN_MS = 1500;

let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();
const deadlines = new Map<number, number>();
const holds = new Map<number, { reasons: Set<MessageToastHold>; left: number }>();

function clearTimer(id: number): void {
  const timer = timers.get(id);
  if (timer) {
    clearTimeout(timer);
    timers.delete(id);
  }
}

function schedule(id: number, ms = TOAST_HOLD_MS): void {
  clearTimer(id);
  // A held toast that gets news counts down from the full time once let go.
  const held = holds.get(id);
  if (held) {
    held.left = Math.max(held.left, ms);
    return;
  }
  deadlines.set(id, Date.now() + ms);
  timers.set(
    id,
    setTimeout(() => {
      useMessageToasts.getState().dismiss(id);
    }, ms),
  );
}

export const useMessageToasts = create<MessageToastState>((set, get) => ({
  toasts: [],
  push: (input) => {
    const now = Date.now();
    const stacked = stackOnto(
      get().toasts.map((row): StackableToast => ({
        id: row.id,
        channelId: row.channelId,
        at: row.at,
        count: row.count,
      })),
      input.channelId,
      now,
    );
    if (stacked) {
      set((state) => ({
        toasts: state.toasts.map((row) =>
          row.id === stacked.id
            ? {
                ...row,
                ...input,
                count: row.count + 1,
                at: now,
              }
            : row,
        ),
      }));
      schedule(stacked.id);
      return;
    }
    const id = nextId++;
    set((state) => ({
      toasts: [
        ...state.toasts.filter((row) => row.channelId !== input.channelId),
        { ...input, id, count: 1, at: now },
      ].slice(-TOAST_MAX),
    }));
    schedule(id);
  },
  dismiss: (id) => {
    clearTimer(id);
    deadlines.delete(id);
    holds.delete(id);
    set((state) => ({ toasts: state.toasts.filter((row) => row.id !== id) }));
  },
  hold: (id, reason) => {
    if (!get().toasts.some((row) => row.id === id)) return;
    const held = holds.get(id);
    if (held) {
      held.reasons.add(reason);
      return;
    }
    clearTimer(id);
    holds.set(id, {
      reasons: new Set([reason]),
      left: (deadlines.get(id) ?? Date.now()) - Date.now(),
    });
  },
  release: (id, reason) => {
    const held = holds.get(id);
    if (!held || !held.reasons.delete(reason) || held.reasons.size > 0) return;
    holds.delete(id);
    schedule(id, Math.max(held.left, RELEASE_MIN_MS));
  },
}));

export function clearMessageToasts(): void {
  for (const id of timers.keys()) clearTimer(id);
  deadlines.clear();
  holds.clear();
  useMessageToasts.setState({ toasts: [] });
}

export function resetMessageToastsForTests(): void {
  clearMessageToasts();
}
