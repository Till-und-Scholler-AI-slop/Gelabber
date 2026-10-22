type Focusable = {
  disabled: boolean;
  isConnected: boolean;
  ownerDocument: { activeElement: unknown; body: unknown };
  closest(selector: string): unknown;
  focus(options?: FocusOptions): void;
};

/**
 * A pending button is disabled, and disabling the focused element sends
 * focus to the page, so after a failed "Anmelden" or "Speichern" the
 * keyboard started over at the top. This remembers that the button had
 * focus and gives it back once the request is over, unless focus has gone
 * somewhere else meanwhile or the button's dialog has closed.
 */
export function pendingFocus() {
  let held: Focusable | null = null;
  return {
    focused(button: Focusable) {
      held = button;
    },
    blurred(button: Focusable) {
      // A blur while disabled is the browser dropping focus, not the user
      // moving on.
      if (!button.disabled) held = null;
    },
    /** Call when the button stops being pending. */
    settled() {
      const button = held;
      if (!button) return;
      const document = button.ownerDocument;
      const active = document.activeElement;
      if (
        button.isConnected &&
        !button.closest("dialog:not([open])") &&
        (!active || active === document.body)
      ) {
        button.focus({ preventScroll: true });
      }
      if (document.activeElement !== button) held = null;
    },
  };
}
