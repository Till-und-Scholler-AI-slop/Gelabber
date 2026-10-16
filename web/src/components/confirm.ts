// In-app replacement for `window.confirm`: themed, and the same in the
// browser, the PWA and the desktop webviews. Rendered by `<ConfirmHost />`.

import { create } from "zustand";

export type ConfirmRequest = {
  title: string;
  message?: string;
  confirmLabel: string;
  tone?: "danger" | "neutral";
};

type Pending = ConfirmRequest & {
  id: number;
  resolve: (ok: boolean) => void;
};

type ConfirmState = {
  pending: Pending | null;
  ask: (request: ConfirmRequest) => Promise<boolean>;
  answer: (id: number, ok: boolean) => void;
};

let nextId = 1;

export const useConfirm = create<ConfirmState>((set, get) => ({
  pending: null,
  ask: (request) =>
    new Promise<boolean>((resolve) => {
      // One question at a time: a new one cancels the one still open.
      get().pending?.resolve(false);
      set({ pending: { ...request, id: nextId++, resolve } });
    }),
  answer: (id, ok) => {
    const pending = get().pending;
    if (!pending || pending.id !== id) return;
    set({ pending: null });
    pending.resolve(ok);
  },
}));

/** Resolves `true` once the user confirms, `false` on cancel or Escape. */
export function confirmAction(request: ConfirmRequest): Promise<boolean> {
  return useConfirm.getState().ask(request);
}
