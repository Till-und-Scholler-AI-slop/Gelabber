// The call's keyboard shortcuts, while in a call: mute and deafen without
// reaching for the call bar.

import { useEffect } from "react";

import { callShortcut } from "./pushToTalk.ts";
import { toggleDeafen, toggleMute } from "./session.ts";
import { useMediaSettings } from "./settings.ts";

export function useCallShortcuts(active: boolean): void {
  useEffect(() => {
    if (!active) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const settings = useMediaSettings.getState();
      const action = callShortcut(
        event,
        settings.voiceInput === "ptt" ? settings.pttKey : null,
      );
      if (!action) return;
      event.preventDefault();
      if (action === "mute") toggleMute();
      else toggleDeafen();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active]);
}
