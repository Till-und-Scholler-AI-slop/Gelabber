// Whether the member list is shown next to a conversation on wide screens.
// Narrow screens use the drawer button instead. Remembered per browser.
//
// A voice channel with pictures running needs the room for them: there the
// list starts closed, until the user opens or closes it there once. That
// choice is remembered on its own, so the list next to chats stays as it
// was.

import { create } from "zustand";

const KEY = "gelabber.members.hidden";
const VIDEO_KEY = "gelabber.members.hiddenWithVideo";

type PanelState = {
  /** Next to conversations and rooms without pictures. */
  hidden: boolean;
  /** Next to pictures: the user's own choice, or null for closed. */
  withVideo: boolean | null;
  /** The page shows a voice channel whose pictures run. */
  video: boolean;
};

function load(key: string): boolean | null {
  try {
    const value = localStorage.getItem(key);
    return value === null ? null : value === "1";
  } catch {
    return null;
  }
}

function save(key: string, hidden: boolean): void {
  try {
    localStorage.setItem(key, hidden ? "1" : "0");
  } catch {
    /* Storage can be blocked; the toggle still works for this tab. */
  }
}

const usePanel = create<PanelState>(() => ({
  hidden: load(KEY) ?? false,
  withVideo: load(VIDEO_KEY),
  video: false,
}));

const hiddenOf = (state: PanelState): boolean =>
  state.video ? (state.withVideo ?? true) : state.hidden;

export function useMemberPanelHidden(): boolean {
  return usePanel(hiddenOf);
}

export function toggleMemberPanel(): void {
  const state = usePanel.getState();
  const hidden = !hiddenOf(state);
  if (state.video) {
    usePanel.setState({ withVideo: hidden });
    save(VIDEO_KEY, hidden);
  } else {
    usePanel.setState({ hidden });
    save(KEY, hidden);
  }
}

/** The voice channel's page says whether its pictures run. */
export function setMemberPanelVideo(video: boolean): void {
  if (usePanel.getState().video !== video) usePanel.setState({ video });
}

/** The list's state outside React. */
export function memberPanelHidden(): boolean {
  return hiddenOf(usePanel.getState());
}
