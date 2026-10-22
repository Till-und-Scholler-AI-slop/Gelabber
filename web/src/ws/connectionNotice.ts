// What the page says about its own connection: nothing while it works,
// "offline" or "reconnecting" while it does not, and a short "restored"
// once it is back. The gateway's quick reconnects stay invisible.

import type { GatewayStatus } from "./client.ts";

export type ConnectionNotice = "offline" | "reconnecting" | "restored" | null;

/** A dropped socket is usually back well within this. */
export const NOTICE_DELAY_MS = 3000;
/** How long "Verbindung wiederhergestellt" stays. */
export const RESTORED_MS = 3000;

type Clock = {
  setTimeout: (run: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
};

export type ConnectionInput = { status: GatewayStatus; online: boolean };

/**
 * Feeds the gateway's status and the browser's online flag through to the
 * notice. A device that says it is offline shows at once; a socket that
 * drops only after `NOTICE_DELAY_MS`.
 */
export function connectionNotices(
  onChange: (notice: ConnectionNotice) => void,
  clock: Clock = {
    setTimeout: (run, ms) => globalThis.setTimeout(run, ms),
    clearTimeout: (handle) =>
      globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
  },
) {
  let notice: ConnectionNotice = null;
  let delay: unknown = null;
  let restored: unknown = null;
  const show = (next: ConnectionNotice) => {
    if (next === notice) return;
    notice = next;
    onChange(next);
  };
  const stopDelay = () => {
    if (delay !== null) clock.clearTimeout(delay);
    delay = null;
  };
  const stopRestored = () => {
    if (restored !== null) clock.clearTimeout(restored);
    restored = null;
  };
  const troubled = () => notice === "offline" || notice === "reconnecting";

  return {
    update({ status, online }: ConnectionInput): void {
      // Signed out: there is no connection to speak of.
      if (status === "idle") {
        stopDelay();
        stopRestored();
        show(null);
        return;
      }
      if (!online) {
        stopDelay();
        stopRestored();
        show("offline");
        return;
      }
      if (status === "open") {
        stopDelay();
        if (!troubled()) return;
        show("restored");
        restored = clock.setTimeout(() => {
          restored = null;
          show(null);
        }, RESTORED_MS);
        return;
      }
      // Connecting or reconnecting, with the device online.
      if (troubled()) {
        show("reconnecting");
        return;
      }
      if (status !== "reconnecting" || delay !== null) return;
      stopRestored();
      show(null);
      delay = clock.setTimeout(() => {
        delay = null;
        show("reconnecting");
      }, NOTICE_DELAY_MS);
    },
    stop(): void {
      stopDelay();
      stopRestored();
    },
  };
}
