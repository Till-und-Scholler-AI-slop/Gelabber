import { useCanGoBack, useRouter } from "@tanstack/react-router";

import { useVoice } from "../voice/session.ts";

/** "Zurück" on the pages outside the workspace (settings, profile): to the
 * previous page, or else to the call that runs, or else home. */
export function BackToApp() {
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const goBack = () => {
    if (canGoBack) {
      router.history.back();
      return;
    }
    const voice = useVoice.getState();
    const serverId = voice.serverId ?? voice.watchServerId;
    const channelId = voice.channelId ?? voice.watchChannelId;
    if (serverId && channelId) {
      void router.navigate({
        to: "/s/$serverId/c/$channelId",
        params: { serverId, channelId },
      });
    } else {
      void router.navigate({ to: "/" });
    }
  };
  return (
    <button type="button" className="settings-back" onClick={goBack}>
      <span aria-hidden="true">←</span> Zurück
    </button>
  );
}
