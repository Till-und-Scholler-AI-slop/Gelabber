// Short-lived notices for things that happen off-form: an optimistic write
// that the server rejected, a copied invite link. Rendered by `<Toasts />`.

import { create } from "zustand";

import { ApiError } from "../api/client.ts";
import { errorMessage, fieldMessages } from "../auth/rules.ts";

export type ToastAction = { label: string; onClick: () => void };

export type Toast = {
  id: number;
  tone: "info" | "error";
  message: string;
  /** One button next to the message, such as "Rückgängig". */
  action?: ToastAction;
};

export type ToastOptions = {
  action?: ToastAction;
  /** Milliseconds on screen; an error stays at least `ERROR_MS`. */
  duration?: number;
};

/** Why a toast is held on screen: the pointer is on it, or focus is in it. */
export type HoldReason = "hover" | "focus";

type ToastState = {
  toasts: Toast[];
  push: (
    tone: Toast["tone"],
    message: string,
    options?: ToastOptions,
  ) => number | null;
  dismiss: (id: number) => void;
  /** Stops the countdown while someone reads or uses the toast. */
  hold: (id: number, reason: HoldReason) => void;
  /** Resumes the countdown once nothing holds the toast any more. */
  release: (id: number, reason: HoldReason) => void;
};

export const INFO_MS = 4000;
/** Long enough to read an error, and a little more (WCAG 2.2.1). */
export const ERROR_MS = 10000;
let nextId = 1;

type Countdown = {
  remaining: number;
  started: number;
  timer: ReturnType<typeof setTimeout> | undefined;
  holds: Set<HoldReason>;
};
const countdowns = new Map<number, Countdown>();

function run(id: number, countdown: Countdown) {
  countdown.started = Date.now();
  countdown.timer = setTimeout(
    () => useToasts.getState().dismiss(id),
    countdown.remaining,
  );
}

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push: (tone, message, options = {}) => {
    const current = useToasts.getState().toasts;
    // A burst of the same notice shows once. One with an action is its own
    // offer: a second "Link widerrufen" undoes a different link.
    if (
      !options.action &&
      current.some((t) => t.tone === tone && t.message === message && !t.action)
    )
      return null;
    if (
      tone === "error" &&
      current.filter((t) => t.tone === "error").length >= 3
    ) {
      return null;
    }
    const id = nextId++;
    const toast: Toast = { id, tone, message };
    if (options.action) toast.action = options.action;
    set((state) => ({ toasts: [...state.toasts, toast] }));
    const duration =
      options.duration ?? (tone === "error" ? ERROR_MS : INFO_MS);
    const countdown: Countdown = {
      remaining: tone === "error" ? Math.max(duration, ERROR_MS) : duration,
      started: 0,
      timer: undefined,
      holds: new Set(),
    };
    countdowns.set(id, countdown);
    run(id, countdown);
    return id;
  },
  dismiss: (id) => {
    clearTimeout(countdowns.get(id)?.timer);
    countdowns.delete(id);
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
  hold: (id, reason) => {
    const countdown = countdowns.get(id);
    if (!countdown) return;
    if (countdown.holds.size === 0) {
      clearTimeout(countdown.timer);
      countdown.timer = undefined;
      countdown.remaining -= Date.now() - countdown.started;
    }
    countdown.holds.add(reason);
  },
  release: (id, reason) => {
    const countdown = countdowns.get(id);
    if (!countdown || !countdown.holds.delete(reason)) return;
    // Back from a hold, there is always a moment left to finish reading.
    countdown.remaining = Math.max(countdown.remaining, 1500);
    if (countdown.holds.size === 0) run(id, countdown);
  },
}));

export function clearToasts(): void {
  for (const countdown of countdowns.values()) clearTimeout(countdown.timer);
  countdowns.clear();
  useToasts.setState({ toasts: [] });
}

/** A short notice; `action` adds a button such as "Rückgängig". */
export function notify(message: string, options?: ToastOptions): void {
  useToasts.getState().push("info", message, options);
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
