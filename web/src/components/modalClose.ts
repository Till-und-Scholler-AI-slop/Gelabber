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
  message: "Deine Eingaben gehen verloren.",
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
  closest?: (selector: string) => {
    querySelector: (selector: string) => unknown;
  } | null;
};

/**
 * Focus goes back to whatever opened the dialog, if it is still shown. An
 * item of a `<details>` menu that closed on the way hands it to the menu's
 * button: the item itself can no longer take focus.
 */
export function restoreFocus(opener: Focusable | null): boolean {
  const menu = opener?.closest?.("details:not([open])");
  const target = menu
    ? (menu.querySelector(":scope > summary") as Focusable | null)
    : opener;
  if (!target?.isConnected || target.getClientRects().length === 0)
    return false;
  target.focus({ preventScroll: true });
  return true;
}

type Surface = { contains: (node: Node | null) => boolean };

/**
 * Whether focus, now at `active`, is still the closing `surface`'s to hand
 * back: it is inside, or nowhere (dropped with the surface's content).
 * Anywhere else, something else has taken it.
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
