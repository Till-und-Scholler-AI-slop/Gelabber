// Input in one Gelabber tab counts for all tabs of the same account in this
// browser (GEL-105): each tab tells the others over a BroadcastChannel, at
// most once per PRESENCE_PULSE_MS, so a tab in the background is not marked
// away while the user works in another one.

import { PRESENCE_PULSE_MS } from "./live.ts";

export type InputShare = {
  /** Input in this tab. */
  post(): void;
  close(): void;
};

export function shareInput(
  userId: string,
  onOtherTab: () => void,
  now: () => number = Date.now,
): InputShare {
  if (typeof BroadcastChannel !== "function") {
    return { post() {}, close() {} };
  }
  const channel = new BroadcastChannel(`gelabber:input:${userId}`);
  channel.onmessage = () => onOtherTab();
  let lastPost = -Infinity;
  return {
    post() {
      const at = now();
      if (at - lastPost < PRESENCE_PULSE_MS) return;
      lastPost = at;
      channel.postMessage("input");
    },
    close() {
      channel.close();
    },
  };
}
