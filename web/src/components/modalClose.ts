// The decisions behind closing a dialog, apart from the DOM so they can be
// tested in node: what counts as a backdrop click, when unsaved input is
// confirmed first, and where focus goes back to.

import { confirmAction, type ConfirmRequest } from "./confirm.ts";

type Box = { top: number; right: number; bottom: number; left: number };

/**
 * A click lands on a modal `<dialog>` itself both on its padding and on its
 * `::backdrop`; only a point outside its box is the backdrop.
 */
export function outsideBox(
  point: { clientX: number; clientY: number },
  box: Box,
): boolean {
  return (
    point.clientX < box.left ||
    point.clientX > box.right ||
    point.clientY < box.top ||
    point.clientY > box.bottom
  );
}

export const DISCARD_CHANGES: ConfirmRequest = {
  title: "Änderungen verwerfen?",
  message: "Was du hier eingegeben hast, geht verloren.",
  confirmLabel: "Verwerfen",
};

/**
 * Esc or "Schließen": closes at once, or with unsaved input only once the
 * user agreed to discard it. Resolves whether the dialog closed.
 */
export async function requestClose({
  dirty,
  onClose,
  confirm = confirmAction,
}: {
  dirty: boolean;
  onClose: () => void;
  confirm?: (request: ConfirmRequest) => Promise<boolean>;
}): Promise<boolean> {
  if (dirty && !(await confirm(DISCARD_CHANGES))) return false;
  onClose();
  return true;
}

/**
 * Whether a click on a modal closes it: a click on the backdrop does,
 * except while there is unsaved input that one stray tap would lose.
 */
export function closesOnBackdrop(
  point: { clientX: number; clientY: number },
  box: Box,
  dirty: boolean,
): boolean {
  return !dirty && outsideBox(point, box);
}

type Focusable = {
  isConnected: boolean;
  getClientRects: () => { length: number };
  focus: (options?: FocusOptions) => void;
};

/** Focus goes back to whatever opened the dialog, if it is still shown. */
export function restoreFocus(opener: Focusable | null): boolean {
  if (!opener?.isConnected || opener.getClientRects().length === 0)
    return false;
  opener.focus({ preventScroll: true });
  return true;
}

type Surface = { contains: (node: Node | null) => boolean };

/**
 * Whether focus, now at `active`, is still the closing `surface`'s to hand
 * back: it is inside, or nowhere (dropped with the surface's content).
 * Anywhere else, something that opened meanwhile has it.
 */
export function focusLeftWith(
  surface: Surface | null,
  active: Element | null,
): boolean {
  return (
    active === null ||
    active.nodeName === "BODY" ||
    (surface?.contains(active) ?? false)
  );
}

// Modal surfaces that are closing but still on screen while they play their
// exit, newest last, each with the control it hands focus back to.
const leaving: { surface: Surface; opener: HTMLElement | null }[] = [];

/**
 * A modal surface plays its exit: until the returned function is called, a
 * dialog that opens meanwhile takes over its opener (see focusOrigin).
 */
export function markLeaving(
  surface: Surface,
  opener: HTMLElement | null,
): () => void {
  const entry = { surface, opener };
  leaving.push(entry);
  return () => {
    const at = leaving.indexOf(entry);
    if (at >= 0) leaving.splice(at, 1);
  };
}

/**
 * Where a dialog opening now returns focus to: the focused control, or,
 * while focus sits in or was dropped by a surface that is still leaving,
 * that surface's own opener, as if it had already closed.
 */
export function focusOrigin(active: Element | null): HTMLElement | null {
  for (let at = leaving.length - 1; at >= 0; at -= 1) {
    const entry = leaving[at]!;
    if (focusLeftWith(entry.surface, active)) return entry.opener;
  }
  return active as HTMLElement | null;
}
