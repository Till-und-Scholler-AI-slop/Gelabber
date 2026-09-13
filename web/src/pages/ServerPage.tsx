// `/s/$serverId`: no channel picked yet — go to the remembered or first text
// channel as soon as the detail is in the cache.

import { Navigate, useParams } from "@tanstack/react-router";

import { useLastChannel } from "../servers/lastChannel.ts";
import { useServer } from "../servers/queries.ts";
import { pickChannel } from "../servers/rows.ts";

export function ServerPage() {
  const { serverId } = useParams({ from: "/workspace/s/$serverId" });
  const { data: server } = useServer(serverId);
  const remembered = useLastChannel((s) => s.byServer[serverId]);

  if (!server) return null;
  const channel = pickChannel(server, remembered);
  if (channel) {
    return (
      <Navigate
        to="/s/$serverId/c/$channelId"
        params={{ serverId, channelId: channel.id }}
        replace
      />
    );
  }
  return (
    <div className="flex flex-1 items-center justify-center p-8 text-center text-neutral-500">
      <p>Dieser Server hat noch keine Kanäle.</p>
    </div>
  );
}
