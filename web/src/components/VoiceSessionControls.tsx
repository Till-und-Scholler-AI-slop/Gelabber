import { Link } from "@tanstack/react-router";

import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
import { retryPlayback, stopWatching, useVoice } from "../voice/session.ts";
import { VoiceControls } from "./VoiceControls.tsx";

/** Active sessions stay controllable independently of the currently open route. */
export function VoiceSessionControls() {
  const voice = useVoice();
  const { data: server } = useServer(voice.serverId ?? undefined);
  if (voice.status !== "joined" && !voice.watching && !voice.playbackBlocked)
    return null;

  return (
    <section
      aria-label="Aktive Medien"
      className="fixed right-3 bottom-3 z-40 flex max-w-[calc(100vw-1.5rem)] flex-col gap-2 rounded-lg border border-neutral-200 bg-white p-3 text-sm shadow-lg dark:border-neutral-700 dark:bg-neutral-900"
    >
      {voice.status === "joined" && voice.serverId && voice.channelId ? (
        <div>
          <Link
            to="/s/$serverId/c/$channelId"
            params={{ serverId: voice.serverId, channelId: voice.channelId }}
            className="block max-w-80 truncate font-medium text-emerald-700 dark:text-emerald-300"
          >
            Verbunden: {voice.channelName ?? "Voice"}
          </Link>
          <VoiceControls compact canGoLive={can(server, "go_live")} />
        </div>
      ) : null}
      {voice.watching && voice.watchServerId && voice.watchChannelId ? (
        <div className="flex items-center justify-between gap-3">
          <Link
            to="/s/$serverId/c/$channelId"
            params={{
              serverId: voice.watchServerId,
              channelId: voice.watchChannelId,
            }}
            className="max-w-60 truncate"
          >
            Zuschauen: {voice.watchChannelName ?? "Live"}
          </Link>
          <button
            type="button"
            onClick={() => stopWatching()}
            className="rounded-md px-2 py-1 text-red-700 hover:bg-red-50 dark:text-red-300 dark:hover:bg-red-950"
          >
            Nicht mehr zuschauen
          </button>
        </div>
      ) : null}
      {voice.playbackBlocked && !voice.deafened ? (
        <div role="status" className="flex items-center gap-3">
          <span>Die Tonwiedergabe ist blockiert.</span>
          <button
            type="button"
            onClick={() => retryPlayback()}
            className="rounded-md bg-neutral-200 px-2 py-1 font-medium dark:bg-neutral-700"
          >
            Ton starten
          </button>
        </div>
      ) : null}
    </section>
  );
}
