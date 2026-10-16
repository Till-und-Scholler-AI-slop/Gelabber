// Which control a modal `<dialog>` focuses when it opens.
//
// `showModal()` focuses the first descendant carrying the `autofocus`
// attribute. On the client React's `autoFocus` prop does not set that
// attribute; it only calls `focus()` at mount, which is a no-op while the
// dialog is still closed. So the attribute is set by hand, and `autoFocus`
// stays for the markup (server render, tests) and for a control that mounts
// inside an already open dialog.

function markAutofocus(element: HTMLElement | null) {
  element?.setAttribute("autofocus", "");
}

export function dialogInitialFocus(on: boolean | undefined) {
  return on ? { autoFocus: true, ref: markAutofocus } : {};
}
