// Whether the phone navigation drawer is open lives in the history entry.
// Opening it adds an entry, so back closes it, and a conversation picked
// from it is pushed on top: back from there opens the channel list again
// instead of walking through earlier conversations. The value is the
// address the drawer was opened over.

import type { HistoryState } from "@tanstack/react-router";
import { useRouter } from "@tanstack/react-router";
import { useCallback, useSyncExternalStore } from "react";

declare module "@tanstack/history" {
  interface HistoryState {
    navigationDrawer?: string;
  }
}

type State = HistoryState & Record<string, unknown>;

/** The address the drawer was opened over, while the entry has it open. */
export function drawerOver(
  state: HistoryState | undefined,
): string | undefined {
  const over = state?.navigationDrawer;
  return typeof over === "string" ? over : undefined;
}

/** The app's own fields of a history state, without the router's keys. */
function ownState(state: HistoryState): State {
  const own: State = {};
  for (const [key, value] of Object.entries(state))
    if (!key.startsWith("__TSR") && key !== "key") own[key] = value;
  return own;
}

/** The state of the entry that opens the drawer over `href`. */
export function openedState(state: HistoryState, href: string): HistoryState {
  return { ...ownState(state), navigationDrawer: href };
}

/** The same entry without the drawer. */
export function closedState(state: HistoryState): HistoryState {
  const own = ownState(state);
  delete own.navigationDrawer;
  return own;
}

/**
 * How to close: going back is right when the entry was opened over the
 * address it shows; after a server was picked inside the drawer the entry
 * shows another one, which must stay, so it is replaced instead.
 */
export function closeStep(
  state: HistoryState | undefined,
  href: string,
): "back" | "replace" | null {
  const over = drawerOver(state);
  if (over === undefined) return null;
  return over === href ? "back" : "replace";
}

// The drawer's own entries never change the address: a navigation blocker
// (an unsaved theme) has nothing to guard there.
const QUIET = { ignoreBlocker: true };

export function useNavigationDrawer() {
  const router = useRouter();
  const { history } = router;
  const over = useSyncExternalStore(
    history.subscribe,
    () => drawerOver(history.location.state),
    () => undefined,
  );
  const show = useCallback(() => {
    const { href, state } = history.location;
    if (drawerOver(state) !== undefined) return;
    history.push(href, openedState(state, href), QUIET);
  }, [history]);
  // Settles once the drawer has closed and handed focus back to its button,
  // so a dialog opened next returns focus there too.
  const hide = useCallback((): Promise<void> => {
    const { href, state } = history.location;
    const step = closeStep(state, href);
    if (step === null) return Promise.resolve();
    return new Promise((resolve) => {
      const stop = history.subscribe(() => {
        stop();
        window.setTimeout(resolve, 0);
      });
      if (step === "back") history.back(QUIET);
      else history.replace(href, closedState(state), QUIET);
    });
  }, [history]);
  return {
    open: over !== undefined,
    /** Link state for a server picked in the open drawer: the drawer stays
     * open on that server's channels. Use with `replace`, in place of the
     * entry that opened it. */
    keepOpen: over === undefined ? undefined : { navigationDrawer: over },
    show,
    hide,
  };
}
