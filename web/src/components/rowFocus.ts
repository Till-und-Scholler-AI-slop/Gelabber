type Focusable = { focus(options?: FocusOptions): void };
type RowList = {
  ownerDocument: { activeElement: unknown; body: unknown };
  querySelectorAll(selectors: string): ArrayLike<Focusable>;
};

/**
 * A row whose action had focus left the list (a device signed out): focus
 * fell to the page, so the keyboard would start over at the top. The action
 * now in its place, the next row's or else the last one's, takes it, unless
 * focus has gone somewhere else meanwhile.
 */
export function focusRowAfterRemoval(list: RowList, index: number): void {
  const document = list.ownerDocument;
  const active = document.activeElement;
  if (active && active !== document.body) return;
  const actions = list.querySelectorAll(
    ":scope > .settings-action-row > :is(button, a)",
  );
  if (actions.length === 0) return;
  actions[Math.min(index, actions.length - 1)]!.focus({ preventScroll: true });
}
