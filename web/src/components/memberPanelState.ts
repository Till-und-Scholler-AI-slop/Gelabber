// Whether the member list is shown next to a conversation on wide screens.
// Narrow screens use the drawer button instead. Remembered per browser.

import { create } from "zustand";

const KEY = "gelabber.members.hidden";

function load(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export const useMemberPanelHidden = create<boolean>(() =>
  typeof localStorage === "undefined" ? false : load(),
);

export function toggleMemberPanel(): void {
  const hidden = !useMemberPanelHidden.getState();
  useMemberPanelHidden.setState(hidden, true);
  try {
    localStorage.setItem(KEY, hidden ? "1" : "0");
  } catch {
    /* Storage can be blocked; the toggle still works for this tab. */
  }
}
