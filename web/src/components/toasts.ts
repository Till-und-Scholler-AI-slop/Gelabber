// Short-lived notices for things that happen off-form: an optimistic write
// that the server rejected, a copied invite link. Rendered by `<Toasts />`.

import { create } from "zustand";

import { ApiError } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";

export type Toast = {
  id: number;
  tone: "info" | "error";
  message: string;
};

type ToastState = {
  toasts: Toast[];
  push: (tone: Toast["tone"], message: string) => void;
  dismiss: (id: number) => void;
};

const TOAST_MS = 4000;
let nextId = 1;

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push: (tone, message) => {
    const id = nextId++;
    set((state) => ({ toasts: [...state.toasts, { id, tone, message }] }));
    setTimeout(() => {
      set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
    }, TOAST_MS);
  },
  dismiss: (id) =>
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));

export function notify(message: string): void {
  useToasts.getState().push("info", message);
}

/** For mutations whose form is already closed: say what went wrong. */
export function notifyError(error: unknown): void {
  if (error instanceof DOMException && error.name === "AbortError") return;
  const message =
    error instanceof ApiError
      ? error.code === "unauthenticated"
        ? null // the session store already bounced the tab to /login
        : errorMessage(error.code)
      : errorMessage("internal");
  if (message) useToasts.getState().push("error", message);
}
