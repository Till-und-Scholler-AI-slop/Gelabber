// Short-lived notices for things that happen off-form: an optimistic write
// that the server rejected, a copied invite link. Rendered by `<Toasts />`.

import { create } from "zustand";

import { ApiError } from "../api/client.ts";
import { errorMessage, fieldMessages } from "../auth/rules.ts";

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
    const current = useToasts.getState().toasts;
    if (current.some((t) => t.tone === tone && t.message === message)) return;
    if (
      tone === "error" &&
      current.filter((t) => t.tone === "error").length >= 3
    ) {
      return;
    }
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

function toastText(error: unknown): string | null {
  if (!(error instanceof ApiError)) {
    return error instanceof Error && error.message
      ? error.message
      : errorMessage("internal");
  }
  if (error.code === "unauthenticated") {
    // The session store already bounced the tab to /login.
    return null;
  }
  if (error.code === "validation_failed") {
    const detail = Object.values(fieldMessages(error.fields)).filter(Boolean);
    if (detail.length > 0) return detail.join(" ");
  }
  return errorMessage(error.code);
}

/** For mutations whose form is already closed: say what went wrong. */
export function notifyError(error: unknown): void {
  if (error instanceof DOMException && error.name === "AbortError") return;
  const message = toastText(error);
  if (message) useToasts.getState().push("error", message);
}
