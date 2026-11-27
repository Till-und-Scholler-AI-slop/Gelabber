// Where the keyboard goes for the composer (GEL-143): into it when a
// conversation opens, and with a letter typed while nothing else has focus.
// The one composer on the page registers its field here, so the shell's key
// handler can reach it.

import { typesText } from "./shellShortcuts.ts";

let field: HTMLTextAreaElement | null = null;

/** Registers the composer's field while it is shown; returns the undo. */
export function registerComposer(next: HTMLTextAreaElement): () => void {
  field = next;
  return () => {
    if (field === next) field = null;
  };
}

/** Moves the keyboard into the composer; false where none is shown. */
export function focusComposer(): boolean {
  if (!field?.isConnected) return false;
  field.focus({ preventScroll: true });
  return document.activeElement === field;
}

/** A field the user writes in: the keyboard is not taken from it. */
function editable(element: Element): boolean {
  return (
    element instanceof HTMLInputElement ||
    element instanceof HTMLTextAreaElement ||
    element instanceof HTMLSelectElement ||
    (element instanceof HTMLElement && element.isContentEditable)
  );
}

function dialogOpen(doc: Document): boolean {
  try {
    return doc.querySelector("dialog:modal") !== null;
  } catch {
    return false;
  }
}

/**
 * Whether opening a conversation may move the keyboard into its composer:
 * with a mouse or keyboard, not on a touch screen, where the on-screen
 * keyboard would open; and not away from a dialog or another field, such as
 * the search.
 */
export function mayFocusOnOpen(target: Window & typeof globalThis): boolean {
  if (target.matchMedia("(pointer: coarse)").matches) return false;
  const doc = target.document;
  if (dialogOpen(doc)) return false;
  const active = doc.activeElement;
  return !active || active === field || !editable(active);
}

/**
 * Whether a letter typed now belongs to the composer: nothing else has the
 * keyboard, that is the page itself or a link of the sidebars that opened
 * the conversation. Buttons, rows and fields keep their keys.
 */
export function typesIntoComposer(doc: Document): boolean {
  if (dialogOpen(doc)) return false;
  const active = doc.activeElement;
  if (!active || active === doc.body || active === doc.documentElement)
    return true;
  return (
    active instanceof HTMLAnchorElement && active.closest("nav, aside") !== null
  );
}

/**
 * Hands the composer a letter typed while nothing has the keyboard; the
 * shell listens with it while the workspace is open. A key something else
 * used first (Kino's K, push-to-talk, a row's own keys) stays where it was:
 * the decision waits until the key has passed every listener, like the
 * shell's shortcuts (useShellShortcuts). Moved there before the key is
 * done, the letter lands in the composer. Returns the undo.
 */
export function forwardTypingToComposer(): () => void {
  const onCapture = (event: KeyboardEvent) => {
    if (
      event.repeat ||
      event.isComposing ||
      !typesText({
        key: event.key,
        altKey: event.altKey,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        shiftKey: event.shiftKey,
        altGraph: event.getModifierState("AltGraph"),
      })
    )
      return;
    const late = (bubbled: KeyboardEvent) => {
      window.removeEventListener("keydown", late);
      if (bubbled !== event || event.defaultPrevented) return;
      if (typesIntoComposer(document)) focusComposer();
    };
    window.addEventListener("keydown", late);
  };
  window.addEventListener("keydown", onCapture, true);
  return () => window.removeEventListener("keydown", onCapture, true);
}
