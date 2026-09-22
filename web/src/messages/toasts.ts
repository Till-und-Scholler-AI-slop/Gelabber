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
  preview: string;
  count: number;
  at: number;
};

type MessageToastState = {
  toasts: MessageToast[];
  push: (input: Omit<MessageToast, "id" | "count" | "at">) => void;
  dismiss: (id: number) => void;
};

let nextId = 1;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function clearTimer(id: number): void {
  const timer = timers.get(id);
  if (timer) {
    clearTimeout(timer);
    timers.delete(id);
  }
}

function schedule(id: number): void {
  clearTimer(id);
  timers.set(
    id,
    setTimeout(() => {
      useMessageToasts.getState().dismiss(id);
    }, TOAST_HOLD_MS),
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
    set((state) => ({ toasts: state.toasts.filter((row) => row.id !== id) }));
  },
}));

export function clearMessageToasts(): void {
  for (const id of timers.keys()) clearTimer(id);
  useMessageToasts.setState({ toasts: [] });
}

export function resetMessageToastsForTests(): void {
  clearMessageToasts();
}
